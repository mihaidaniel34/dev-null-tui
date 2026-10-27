use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use tokio::sync::mpsc::UnboundedSender;

use crate::api::{Api, Log, api_error};
use crate::attach::{self, Attachment};
use crate::auth::{Authorization, DeviceKey};
use crate::callback;
use crate::config::{self, Config};
use crate::model::*;
use crate::remotes::{self, RemoteMsg};
use crate::system;

pub const MAX_BODY: usize = 4000;
const NEW_COMMITS_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Screen {
    Log(Log),
    Notifications(Filter),
    /// What the user starred, which is how commits are kept since saving went away.
    Starred,
    /// A profile, on one of its tabs.
    Profile(String, Tab),
    Thread(String),
    Settings,
    /// The inbox of Remotes, the direct messages.
    Remotes,
    /// A conversation in Remotes, by the partner's handle.
    Remote(String),
}

impl Screen {
    pub fn title(&self) -> String {
        match self {
            Screen::Log(log) => log.title().into(),
            Screen::Notifications(_) => "Notifications".into(),
            Screen::Starred => "Starred".into(),
            Screen::Profile(username, _) => format!("@{username}"),
            Screen::Thread(_) => "Commit".into(),
            Screen::Settings => "Settings".into(),
            Screen::Remotes => "Remotes".into(),
            Screen::Remote(handle) => format!("@{handle}"),
        }
    }
}

/// The web's notification filters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Filter {
    All,
    Replies,
    Mentions,
    Quotes,
    Stars,
    Follows,
    Moderation,
}

impl Filter {
    pub const ALL: [Filter; 7] = [
        Filter::All,
        Filter::Replies,
        Filter::Mentions,
        Filter::Quotes,
        Filter::Stars,
        Filter::Follows,
        Filter::Moderation,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Replies => "Replies",
            Filter::Mentions => "Mentions",
            Filter::Quotes => "Quotes",
            Filter::Stars => "Stars",
            Filter::Follows => "Follows",
            Filter::Moderation => "Moderation",
        }
    }

    pub fn query(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Replies => "replies",
            Filter::Mentions => "mentions",
            Filter::Quotes => "quotes",
            Filter::Stars => "stars",
            Filter::Follows => "follows",
            Filter::Moderation => "moderation",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tab {
    Commits,
    Replies,
    Followers,
    Following,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Commits, Tab::Replies, Tab::Followers, Tab::Following];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Commits => "Commits",
            Tab::Replies => "Replies",
            Tab::Followers => "Followers",
            Tab::Following => "Following",
        }
    }

    pub fn is_people(self) -> bool {
        matches!(self, Tab::Followers | Tab::Following)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Normal,
    /// A commit the focused one replies to, above it in a thread.
    Ancestor,
    Focus,
    Reply,
}

#[derive(Clone, Debug)]
pub enum Item {
    Commit(Box<Commit>, Role),
    Notification(Box<Notification>),
    User(Box<User>),
}

impl Item {
    pub fn commit(&self) -> Option<&Commit> {
        match self {
            Item::Commit(c, _) => Some(c),
            Item::Notification(n) => n.commit.as_ref(),
            Item::User(_) => None,
        }
    }
}

#[derive(Default)]
pub struct Feed {
    pub items: Vec<Item>,
    pub selected: usize,
    pub cursor: Option<String>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
    /// Why there is nothing to show, such as a log that needs signing in.
    pub notice: Option<String>,
    pub scroll: usize,
    /// Where each item starts, in rendered lines, for mouse clicks.
    pub offsets: Vec<(usize, usize)>,
    /// Commits whose replies are all shown (`o`), and ones whose reply preview is hidden (`o` again).
    pub expanded: HashSet<String>,
    pub collapsed: HashSet<String>,
    pub replies: HashMap<String, Vec<Commit>>,
    pub profile: Option<Profile>,
    pub new_count: u64,
    pub checked_at: Option<Instant>,
}

pub enum Loaded {
    Commits(Vec<Commit>, Option<String>),
    Notifications(Vec<Notification>, Option<String>),
    Thread {
        ancestors: Vec<Commit>,
        focus: Commit,
        replies: Vec<Commit>,
        cursor: Option<String>,
        notice: Option<String>,
    },
    Profile(Profile, Vec<Item>, Option<String>),
    People(Vec<User>, Option<String>),
    /// Nothing to load, and why.
    Notice(String),
}

pub enum Msg {
    Loaded {
        screen: Screen,
        more: bool,
        result: Result<Loaded, String>,
    },
    Replies {
        screen: Screen,
        id: String,
        result: Result<Vec<Commit>, String>,
    },
    Me(Result<CurrentUser, ApiFailure>),
    CommitUpdated(Commit),
    CommitDeleted(String),
    Posted(Result<Commit, String>),
    Followed {
        username: String,
        on: bool,
        result: Result<(), String>,
    },
    NewCommits {
        screen: Screen,
        count: u64,
    },
    SignedIn(Result<CurrentUser, String>),
    Toast(String, bool),
    /// The signed-in user, after changing a setting.
    MeUpdated(Result<CurrentUser, String>, String),
    Muted {
        username: String,
        on: bool,
    },
    Blocked {
        username: String,
        on: bool,
    },
    /// A picture is ready to send to the terminal, or couldn't be had.
    Picture(crate::gfx::Key, Option<Vec<u8>>),
    Remote(RemoteMsg),
}

pub struct ApiFailure {
    pub code: Option<String>,
    pub message: String,
}

#[derive(Clone)]
pub enum ComposeMode {
    New,
    Reply(Box<Commit>),
    Quote(Box<Commit>),
}

pub struct Composer {
    pub mode: ComposeMode,
    pub text: Vec<char>,
    pub images: Vec<Attachment>,
    pub cursor: usize,
    pub preview: bool,
    pub sending: bool,
    pub error: Option<String>,
    /// Which suggestion is picked while completing an @mention.
    pub mention_index: usize,
}

impl Composer {
    /// The @mention being typed at the cursor: where its @ is, and what follows it so far.
    pub fn mention(&self) -> Option<(usize, String)> {
        let mut start = self.cursor;
        while start > 0 && (self.text[start - 1].is_ascii_alphanumeric() || matches!(self.text[start - 1], '-' | '_')) {
            start -= 1;
        }
        let at = start.checked_sub(1)?;
        // An email address isn't a mention.
        if self.text[at] != '@' || (at > 0 && self.text[at - 1].is_alphanumeric()) {
            return None;
        }
        Some((at, self.text[start..self.cursor].iter().collect()))
    }

    pub fn body(&self) -> String {
        self.text.iter().collect::<String>().trim().to_string()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Go,
    Username,
    SignInCode,
    LaneColor,
}

pub struct Input {
    pub kind: InputKind,
    pub text: Vec<char>,
    pub cursor: usize,
    pub error: Option<String>,
}

pub enum Confirm {
    Delete(Commit),
    SignOut,
    Mute(String, bool),
    Block(String, bool),
    /// Picks a reason, by number, then reports.
    Report(Commit),
}

pub const REPORT_REASONS: [(&str, &str); 4] = [
    ("spam", "Spam"),
    ("harassment", "Harassment or hate"),
    ("unsafe", "Unsafe or illegal"),
    ("other", "Something else"),
];

/// A commit to send through /api/cli/commits, which the main loop runs, since ssh-keygen may need the terminal.
pub struct CliJob {
    pub reply_to: Option<String>,
    pub body: String,
    pub images: Vec<Attachment>,
}

pub struct Viewer {
    pub images: Vec<Image>,
    pub index: usize,
    pub author: String,
}

pub struct SignIn {
    pub key: Arc<DeviceKey>,
    pub authorization: Authorization,
    pub redeeming: bool,
    /// devnull:// is ours while this is alive, so the browser can hand the link back.
    pub handler: Option<callback::Handler>,
}

pub struct App {
    pub api: Api,
    pub config: Config,
    pub tx: UnboundedSender<Msg>,
    pub me: Option<CurrentUser>,
    pub stack: Vec<Screen>,
    pub feeds: HashMap<Screen, Feed>,
    pub log: Log,
    pub composer: Option<Composer>,
    pub draft: Option<Vec<char>>,
    pub draft_images: Vec<Attachment>,
    pub input: Option<Input>,
    pub confirm: Option<Confirm>,
    pub help: bool,
    /// The profile's achievements, opened with A.
    pub achievements: bool,
    /// Whether the terminal's background is light, when it said.
    pub detected_light: Option<bool>,
    /// A commit's images, full size, opened with v.
    pub viewer: Option<Viewer>,
    /// The lane being picked in Settings, 0 to 5, or 6 for Automatic, before it's saved.
    pub lane_pick: Option<usize>,
    pub help_scroll: usize,
    pub pending_g: bool,
    pub toast: Option<(String, bool, Instant)>,
    pub busy: Option<String>,
    pub tick: usize,
    pub quit: bool,
    pub cli_job: Option<CliJob>,
    pub sign_in: Option<SignIn>,
    /// Where the feed was drawn, for mouse clicks.
    pub feed_area: ratatui::layout::Rect,
    pub remotes: remotes::Remotes,
}

impl App {
    pub fn new(api: Api, config: Config, tx: UnboundedSender<Msg>) -> Self {
        let mut app = App {
            api,
            config,
            tx,
            me: None,
            stack: Vec::new(),
            feeds: HashMap::new(),
            log: Log::Latest,
            composer: None,
            draft: config::load_draft().map(|d| d.chars().collect()),
            draft_images: Vec::new(),
            input: None,
            confirm: None,
            help: false,
            achievements: false,
            lane_pick: None,
            viewer: None,
            detected_light: None,
            help_scroll: 0,
            pending_g: false,
            toast: None,
            busy: None,
            tick: 0,
            quit: false,
            cli_job: None,
            sign_in: None,
            feed_area: Default::default(),
            remotes: Default::default(),
        };

        if app.signed() {
            app.load_me();
            app.navigate(Screen::Log(Log::Latest));
        } else if app.config.username.is_some() {
            app.log = Log::Following;
            app.navigate(Screen::Log(Log::Following));
        } else {
            app.navigate(Screen::Settings);
        }
        app
    }

    pub fn signed(&self) -> bool {
        self.api.is_signed()
    }

    pub fn username(&self) -> Option<String> {
        self.me
            .as_ref()
            .map(|m| m.user.username.clone())
            .or_else(|| self.config.username.clone())
    }

    pub fn screen(&self) -> &Screen {
        self.stack.last().expect("a screen")
    }

    pub fn feed(&self) -> Option<&Feed> {
        self.feeds.get(self.screen())
    }

    pub fn feed_mut(&mut self) -> Option<&mut Feed> {
        let screen = self.screen().clone();
        self.feeds.get_mut(&screen)
    }

    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = Msg> + Send + 'static,
    {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(future.await);
        });
    }

    pub fn toast(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), false, Instant::now()));
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), true, Instant::now()));
    }

    pub fn load_me(&self) {
        let api = self.api.clone();
        self.spawn(async move {
            Msg::Me(api.me().await.map_err(|e| ApiFailure {
                code: api_error(&e).and_then(|a| a.code.clone()),
                message: e.to_string(),
            }))
        });
    }

    // Navigation ------------------------------------------------------------------------------------------------

    pub fn navigate(&mut self, screen: Screen) {
        let remote = matches!(screen, Screen::Remotes | Screen::Remote(_));
        if remote && !self.signed() {
            self.error("Sign in to use Remotes: g s");
            return;
        }
        if let Screen::Log(log) = screen {
            self.log = log;
            // Log tabs replace each other rather than stacking up.
            if matches!(self.stack.last(), Some(Screen::Log(_))) {
                self.stack.pop();
            }
        }
        if self.stack.last() != Some(&screen) {
            self.stack.push(screen.clone());
        }
        if self.stack.len() > 50 {
            self.stack.remove(0);
        }

        if remote {
            self.remotes_opened(&screen);
            return;
        }

        let feed = self.feeds.entry(screen.clone()).or_default();
        if !feed.loaded && !feed.loading {
            self.load(screen, false);
        }
    }

    pub fn back(&mut self) {
        if self.stack.len() > 1
            && let Some(left) = self.stack.pop()
        {
            self.remotes_left(&left);
        }
    }

    pub fn refresh(&mut self) {
        let screen = self.screen().clone();
        if screen == Screen::Settings {
            return;
        }
        if matches!(screen, Screen::Remotes | Screen::Remote(_)) {
            return self.remotes_refresh();
        }
        if let Some(feed) = self.feeds.get_mut(&screen) {
            feed.new_count = 0;
            feed.error = None;
        }
        self.load(screen, false);
    }

    pub fn load(&mut self, screen: Screen, more: bool) {
        let signed = self.signed();
        let guest_name = self.config.username.clone();
        let feed = self.feeds.entry(screen.clone()).or_default();
        if feed.loading || (more && feed.cursor.is_none()) {
            return;
        }
        let cursor = if more { feed.cursor.clone() } else { None };
        feed.loading = true;
        feed.error = None;

        let api = self.api.clone();
        let target = screen.clone();
        self.spawn(async move {
            let result = fetch(api, target.clone(), cursor, signed, guest_name).await;
            Msg::Loaded {
                screen: target,
                more,
                result,
            }
        });
    }

    fn maybe_load_more(&mut self) {
        let screen = self.screen().clone();
        if let Some(feed) = self.feeds.get(&screen)
            && feed.cursor.is_some()
            && !feed.loading
            && feed.selected + 4 >= feed.items.len()
        {
            self.load(screen, true);
        }
    }

    // Messages --------------------------------------------------------------------------------------------------

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Loaded { screen, more, result } => {
                let feed = self.feeds.entry(screen).or_default();
                feed.loading = false;
                feed.loaded = true;
                match result {
                    Err(e) => feed.error = Some(e),
                    Ok(loaded) => apply(feed, loaded, more),
                }
            }
            Msg::Replies { screen, id, result } => match result {
                Ok(replies) => {
                    if let Some(feed) = self.feeds.get_mut(&screen) {
                        feed.replies.insert(id, replies);
                    }
                }
                Err(e) => self.error(e),
            },
            Msg::Me(Ok(me)) => {
                if self.config.username.as_deref() != Some(me.user.username.as_str()) {
                    self.config.username = Some(me.user.username.clone());
                    let _ = self.config.save();
                }
                self.remotes.unread = me.remotes.as_ref().map_or(0, |r| r.unread_count);
                self.me = Some(me);
            }
            Msg::Me(Err(failure)) => {
                if failure.code.as_deref() == Some("device_not_approved") {
                    self.sign_out_locally();
                    self.error("This terminal was signed out on the web. Sign in again in Settings (g s).");
                } else {
                    self.error(failure.message);
                }
            }
            Msg::CommitUpdated(commit) => self.update_commit(commit),
            Msg::CommitDeleted(id) => {
                for feed in self.feeds.values_mut() {
                    feed.items.retain(|item| item.commit().is_none_or(|c| c.id != id));
                    feed.selected = feed.selected.min(feed.items.len().saturating_sub(1));
                }
                self.toast("Deleted. It's in /dev/null now.");
            }
            Msg::Posted(result) => self.on_posted(result),
            Msg::Followed { username, on, result } => match result {
                Ok(()) => {
                    for feed in self.feeds.values_mut() {
                        for item in feed.items.iter_mut() {
                            if let Item::User(user) = item
                                && user.username == username
                            {
                                user.is_followed = Some(on);
                            }
                        }
                    }
                    for (screen, feed) in self.feeds.iter_mut() {
                        if matches!(screen, Screen::Profile(u, _) if *u == username)
                            && let Some(profile) = feed.profile.as_mut()
                        {
                            profile.is_following = Some(on);
                            if let Some(n) = profile.data.followers_count.as_mut() {
                                *n = if on { *n + 1 } else { n.saturating_sub(1) };
                            }
                        }
                    }
                    self.feeds.remove(&Screen::Log(Log::Following));
                    self.toast(if on {
                        format!("Following @{username}")
                    } else {
                        format!("Unfollowed @{username}")
                    });
                }
                Err(e) => self.error(e),
            },
            Msg::NewCommits { screen, count } => {
                if let Some(feed) = self.feeds.get_mut(&screen) {
                    feed.new_count = count;
                }
            }
            Msg::SignedIn(result) => self.on_signed_in(result),
            Msg::Toast(message, error) => self.toast = Some((message, error, Instant::now())),
            Msg::Picture(key, png) => crate::gfx::loaded(key, png),
            Msg::Remote(msg) => self.on_remote_msg(msg),
            Msg::MeUpdated(result, done) => {
                self.busy = None;
                match result {
                    Ok(me) => {
                        self.lane_pick = None;
                        self.recolor(&me.user);
                        self.me = Some(me);
                        self.toast(done);
                    }
                    Err(e) => self.error(e),
                }
            }
            Msg::Muted { username, on } => {
                for (screen, feed) in self.feeds.iter_mut() {
                    if matches!(screen, Screen::Profile(u, _) if *u == username)
                        && let Some(profile) = feed.profile.as_mut()
                    {
                        profile.is_muting = Some(on);
                    }
                }
                // Logs leave a muted developer out, so they're read again.
                self.feeds.retain(|screen, _| !matches!(screen, Screen::Log(_)));
            }
            Msg::Blocked { username, on } => {
                // Blocking ends the follows both ways and hides each from the other: start the logs over, and leave.
                self.feeds
                    .retain(|screen, _| !matches!(screen, Screen::Log(_) | Screen::Profile(..)));
                self.stack
                    .retain(|screen| !matches!(screen, Screen::Profile(u, _) if *u == username));
                if self.stack.is_empty() {
                    self.stack.push(Screen::Log(self.log));
                }
                let screen = self.screen().clone();
                self.feeds.remove(&screen);
                self.navigate(screen);
                self.toast(if on {
                    format!("Blocked @{username}.")
                } else {
                    format!("Unblocked @{username}.")
                });
            }
        }
    }

    fn update_commit(&mut self, commit: Commit) {
        let patch = |c: &mut Commit| {
            if c.id == commit.id {
                c.merge(commit.clone());
            }
        };
        for feed in self.feeds.values_mut() {
            for item in feed.items.iter_mut() {
                match item {
                    Item::Commit(c, _) => patch(c),
                    Item::Notification(n) => {
                        if let Some(c) = n.commit.as_mut() {
                            patch(c)
                        }
                    }
                    Item::User(_) => {}
                }
            }
            for replies in feed.replies.values_mut() {
                replies.iter_mut().for_each(patch);
            }
        }
    }

    fn on_posted(&mut self, result: Result<Commit, String>) {
        let Some(composer) = self.composer.as_mut() else { return };
        composer.sending = false;
        self.busy = None;
        match result {
            Err(e) => composer.error = Some(e),
            Ok(commit) => {
                let pending = commit.status.as_deref() == Some("pending");
                let mode = composer.mode.clone();
                self.composer = None;
                self.draft = None;
                config::save_draft(None);
                self.draft_images.clear();
                self.toast(if pending {
                    "Committed. Pending review: only you can see it until it's approved."
                } else {
                    "Committed."
                });

                match mode {
                    ComposeMode::Reply(parent) => {
                        if let Some(feed) = self.feeds.get_mut(&Screen::Thread(parent.id.clone())) {
                            feed.items.push(Item::Commit(Box::new(commit), Role::Reply));
                        }
                        let mut parent = *parent;
                        parent.replies_count = Some(parent.replies_count.unwrap_or(0) + 1);
                        self.update_commit(parent);
                    }
                    _ => {
                        for log in [Log::Latest, Log::Following] {
                            if let Some(feed) = self.feeds.get_mut(&Screen::Log(log)) {
                                feed.items.insert(0, Item::Commit(Box::new(commit.clone()), Role::Normal));
                                feed.selected = 0;
                                feed.scroll = 0;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);

        // The browser handed the sign-in link over: finish signing in without any pasting.
        if self.sign_in.as_ref().is_some_and(|s| s.handler.is_some() && !s.redeeming)
            && let Some(link) = callback::take()
        {
            let input = self.input.get_or_insert(Input {
                kind: InputKind::SignInCode,
                text: Vec::new(),
                cursor: 0,
                error: None,
            });
            input.text = link.chars().collect();
            input.cursor = input.text.len();
            input.error = None;
            self.redeem(link);
        }

        if let Some((_, error, at)) = &self.toast
            && at.elapsed() > Duration::from_secs(if *error { 8 } else { 4 })
        {
            self.toast = None;
        }

        self.remotes_tick();

        // Count what's new on the log, as the web does, about once a minute.
        if let Screen::Log(log) = self.screen().clone() {
            if !self.signed() || log == Log::Trending {
                return;
            }
            let screen = Screen::Log(log);
            let Some(feed) = self.feeds.get_mut(&screen) else { return };
            if feed.loading || feed.checked_at.is_some_and(|at| at.elapsed() < NEW_COMMITS_EVERY) {
                return;
            }
            feed.checked_at = Some(Instant::now());
            let Some(first) = feed.items.first().and_then(|i| i.commit()).map(|c| c.id.clone()) else {
                return;
            };
            let api = self.api.clone();
            self.spawn(async move {
                let count = api.new_commits_since(log, &first).await.unwrap_or(0);
                Msg::NewCommits { screen, count }
            });
        }
    }

    // Actions ---------------------------------------------------------------------------------------------------

    pub fn selected_commit(&self) -> Option<Commit> {
        let feed = self.feed()?;
        feed.items.get(feed.selected)?.commit().filter(|c| !c.is_tombstone()).cloned()
    }

    fn needs_sign_in(&mut self, what: &str) -> bool {
        if self.signed() {
            return false;
        }
        self.error(format!("Sign in to {what}: g s"));
        true
    }

    fn move_by(&mut self, delta: isize) {
        if self.remotes_scroll(delta) {
            return;
        }
        if let Some(feed) = self.feed_mut() {
            if feed.items.is_empty() {
                return;
            }
            let last = feed.items.len() as isize - 1;
            feed.selected = (feed.selected as isize + delta).clamp(0, last) as usize;
        }
        self.maybe_load_more();
    }

    fn open_selected(&mut self) {
        let Some(feed) = self.feed() else { return };
        let Some(item) = feed.items.get(feed.selected) else { return };
        match item {
            Item::User(user) => {
                let username = user.username.clone();
                self.navigate(Screen::Profile(username, Tab::Commits));
            }
            Item::Notification(n) if n.kind == "followed" => {
                if let Some(actor) = &n.actor {
                    let username = actor.username.clone();
                    self.navigate(Screen::Profile(username, Tab::Commits));
                }
            }
            // On an open commit, enter replies to it, as the web's reply box under it does. `a` opens the author.
            Item::Commit(_, Role::Focus) => {
                if let Some(commit) = self.selected_commit() {
                    self.compose(ComposeMode::Reply(Box::new(commit)));
                }
            }
            _ => {
                if let Some(commit) = self.selected_commit() {
                    self.navigate(Screen::Thread(commit.id));
                }
            }
        }
    }

    fn open_author(&mut self) {
        let feed = self.feed();
        let author = feed.and_then(|f| f.items.get(f.selected)).and_then(|item| match item {
            Item::Notification(n) => n.actor.clone(),
            Item::Commit(c, _) => c.author.clone(),
            Item::User(u) => Some((**u).clone()),
        });
        if let Some(author) = author {
            self.navigate(Screen::Profile(author.username, Tab::Commits));
        }
    }

    fn toggle_replies(&mut self) {
        let Some(commit) = self.selected_commit() else { return };
        let screen = self.screen().clone();
        let signed = self.signed();
        let Some(feed) = self.feeds.get_mut(&screen) else { return };

        if feed.expanded.remove(&commit.id) {
            feed.collapsed.insert(commit.id.clone());
        } else {
            feed.collapsed.remove(&commit.id);
            feed.expanded.insert(commit.id.clone());
            if signed && commit.replies_count.unwrap_or(0) > 0 && !feed.replies.contains_key(&commit.id) {
                let api = self.api.clone();
                let id = commit.id.clone();
                self.spawn(async move {
                    let result = api.replies(&id, None).await.map(|p| p.data).map_err(|e| e.to_string());
                    Msg::Replies { screen, id, result }
                });
            } else if !signed && commit.replies_count.unwrap_or(0) > 0 {
                self.toast("Sign in to read replies: g s");
            }
        }
    }

    fn star(&mut self) {
        if self.needs_sign_in("star commits") {
            return;
        }
        let Some(commit) = self.selected_commit() else { return };
        let on = !commit.starred.unwrap_or(false);

        let mut optimistic = commit.clone();
        optimistic.starred = Some(on);
        optimistic.stars_count = Some(if on {
            commit.stars_count.unwrap_or(0) + 1
        } else {
            commit.stars_count.unwrap_or(1) - 1
        });
        self.update_commit(optimistic);
        self.feeds.remove(&Screen::Starred);

        let api = self.api.clone();
        self.spawn(async move {
            match api.star(&commit.id, on).await {
                Ok(fresh) => Msg::CommitUpdated(fresh),
                Err(e) => {
                    let _ = e;
                    Msg::CommitUpdated(commit)
                }
            }
        });
    }

    fn pastel(&mut self) {
        if self.needs_sign_in("give pastéis") {
            return;
        }
        if !self.me.as_ref().is_some_and(|m| m.pasteis_mode) {
            self.toast("Pastéis mode is off. Turn it on from the web.");
            return;
        }
        let Some(commit) = self.selected_commit() else { return };
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.spawn(async move {
            match api.pastel(&commit.id).await {
                Ok(fresh) => {
                    let _ = tx.send(Msg::Toast("One pastel de nata, delivered.".into(), false));
                    Msg::CommitUpdated(fresh)
                }
                Err(e) => Msg::Toast(e.to_string(), true),
            }
        });
        if let Some(me) = self.me.as_mut() {
            me.pasteis_remaining = me.pasteis_remaining.map(|n| n.saturating_sub(1));
        }
    }

    fn follow(&mut self) {
        // In a list of people, the selected one.
        let selected_user = self.feed().and_then(|f| f.items.get(f.selected)).and_then(|item| match item {
            Item::User(u) => Some((**u).clone()),
            _ => None,
        });
        if let Some(user) = selected_user {
            if self.needs_sign_in("follow people") || self.username().as_deref() == Some(user.username.as_str()) {
                return;
            }
            let on = !user.is_followed.unwrap_or(false);
            let (api, username) = (self.api.clone(), user.username);
            self.spawn(async move {
                let result = api.follow(&username, on).await.map_err(|e| e.to_string());
                Msg::Followed { username, on, result }
            });
            return;
        }
        let Screen::Profile(username, _) = self.screen().clone() else {
            self.toast("Open a profile to follow someone (enter on their name, or /).");
            return;
        };
        if self.needs_sign_in("follow people") {
            return;
        }
        if self.username().as_deref() == Some(username.as_str()) {
            return;
        }
        let on = !self
            .feed()
            .and_then(|f| f.profile.as_ref())
            .and_then(|p| p.is_following)
            .unwrap_or(false);
        let api = self.api.clone();
        self.spawn(async move {
            let result = api.follow(&username, on).await.map_err(|e| e.to_string());
            Msg::Followed { username, on, result }
        });
    }

    /// Opens the selected commit's images full size, or in the browser where the terminal can't show pictures.
    fn view_images(&mut self) {
        let Some(commit) = self.selected_commit() else { return };
        let images: Vec<Image> = commit.images.unwrap_or_default().into_iter().filter(|i| i.url.is_some()).collect();
        if images.is_empty() {
            self.toast("That commit has no images.");
            return;
        }
        if !crate::gfx::enabled() {
            if let Some(url) = &images[0].url
                && system::open(url)
            {
                self.toast("Opened the image in your browser. Pictures show in kitty and Ghostty.");
            }
            return;
        }
        let author = commit.author.map(|a| a.username).unwrap_or_default();
        self.viewer = Some(Viewer { images, index: 0, author });
    }

    fn ask_mute_or_block(&mut self, block: bool) {
        let Screen::Profile(username, _) = self.screen().clone() else {
            self.toast(if block {
                "Open a profile to block someone."
            } else {
                "Open a profile to mute someone."
            });
            return;
        };
        if self.needs_sign_in(if block { "block people" } else { "mute people" }) || self.username().as_deref() == Some(username.as_str()) {
            return;
        }
        let profile = self.feed().and_then(|f| f.profile.as_ref());
        if block {
            self.confirm = Some(Confirm::Block(username, true));
        } else {
            let muting = profile.and_then(|p| p.is_muting).unwrap_or(false);
            self.confirm = Some(Confirm::Mute(username, !muting));
        }
    }

    fn mute(&mut self, username: String, on: bool) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.spawn(async move {
            match api.mute(&username, on).await {
                Ok(()) => {
                    let _ = tx.send(Msg::Muted {
                        username: username.clone(),
                        on,
                    });
                    Msg::Toast(
                        if on {
                            format!("Muted @{username}. They won't know.")
                        } else {
                            format!("Unmuted @{username}.")
                        },
                        false,
                    )
                }
                Err(e) => Msg::Toast(e.to_string(), true),
            }
        });
    }

    fn block(&mut self, username: String, on: bool) {
        let api = self.api.clone();
        self.spawn(async move {
            match api.block(&username, on).await {
                Ok(()) => Msg::Blocked { username, on },
                Err(e) => Msg::Toast(e.to_string(), true),
            }
        });
    }

    fn report(&mut self, commit: Commit, reason: &'static str) {
        let api = self.api.clone();
        self.spawn(async move {
            match api.report(&commit.id, reason).await {
                Ok(()) => Msg::Toast("Reported. Thanks: a moderator will take a look.".into(), false),
                Err(e) => Msg::Toast(e.to_string(), true),
            }
        });
    }

    fn copy_link(&mut self) {
        if let Screen::Profile(username, _) = self.screen().clone()
            && self.selected_commit().is_none()
        {
            let url = format!("{}/@{}", self.api.base, username);
            system::copy(&url);
            self.toast(format!("Copied {url}"));
            return;
        }
        if *self.screen() == Screen::Settings {
            if let Some(sign_in) = &self.sign_in {
                system::copy(&sign_in.authorization.url);
                self.toast("Copied the sign-in link.");
            }
            return;
        }
        if let Some(url) = self.selected_commit().and_then(|c| c.html_url) {
            system::copy(&url);
            self.toast(format!("Copied {url}"));
        }
    }

    fn open_in_browser(&mut self) {
        let url = match self.screen().clone() {
            Screen::Settings => self.sign_in.as_ref().map(|s| s.authorization.url.clone()),
            Screen::Profile(username, _) if self.selected_commit().is_none() => Some(format!("{}/@{}", self.api.base, username)),
            _ => self.selected_commit().and_then(|c| c.html_url),
        };
        if let Some(url) = url {
            if system::open(&url) {
                self.toast("Opened in your browser.");
            } else {
                self.error("Could not open a browser. Copy the link with y.");
            }
        }
    }

    fn compose(&mut self, mode: ComposeMode) {
        if let ComposeMode::Quote(_) = mode
            && self.needs_sign_in("quote commits")
        {
            return;
        }
        let (text, images) = match mode {
            ComposeMode::New => (self.draft.take().unwrap_or_default(), std::mem::take(&mut self.draft_images)),
            ComposeMode::Reply(_) | ComposeMode::Quote(_) => (Vec::new(), Vec::new()),
        };
        let cursor = text.len();
        self.composer = Some(Composer {
            mode,
            text,
            images,
            cursor,
            preview: false,
            sending: false,
            error: None,
            mention_index: 0,
        });
    }

    /// Developers seen anywhere in the app, for completing @mentions: whoever starts with what's typed first.
    pub fn mention_candidates(&self, prefix: &str) -> Vec<User> {
        let mut seen: HashMap<String, User> = HashMap::new();
        let mut add = |u: &User| {
            if !u.username.is_empty() {
                seen.entry(u.username.to_lowercase()).or_insert_with(|| u.clone());
            }
        };
        for feed in self.feeds.values() {
            for item in &feed.items {
                match item {
                    Item::Commit(c, _) => {
                        c.author.iter().for_each(&mut add);
                        c.quoted_commit.iter().filter_map(|q| q.author.as_ref()).for_each(&mut add);
                        c.reply_preview
                            .iter()
                            .flat_map(|p| &p.data)
                            .filter_map(|r| r.author.as_ref())
                            .for_each(&mut add);
                    }
                    Item::Notification(n) => n.actor.iter().for_each(&mut add),
                    Item::User(u) => add(u),
                }
            }
            if let Some(p) = &feed.profile {
                add(&p.data);
            }
        }
        let me = self.username().unwrap_or_default().to_lowercase();
        let prefix = prefix.to_lowercase();
        let mut matches: Vec<User> = seen
            .into_values()
            .filter(|u| u.username.to_lowercase() != me)
            .filter(|u| {
                prefix.is_empty() || u.username.to_lowercase().starts_with(&prefix) || u.display_name().to_lowercase().contains(&prefix)
            })
            .collect();
        matches.sort_by_key(|u| (!u.username.to_lowercase().starts_with(&prefix), u.username.to_lowercase()));
        matches.truncate(5);
        matches
    }

    fn submit(&mut self) {
        let signed = self.signed();
        let Some(composer) = self.composer.as_mut() else { return };
        if composer.sending {
            return;
        }
        let body = composer.body();
        if body.is_empty() && composer.images.is_empty() {
            composer.error = Some("Aborting commit due to empty commit message.".into());
            return;
        }
        if !composer.images.is_empty() && matches!(composer.mode, ComposeMode::Quote(_)) {
            composer.error = Some("A quote can't carry images yet. Commit them on their own.".into());
            return;
        }
        if body.chars().count() > MAX_BODY {
            composer.error = Some(format!("A commit fits {MAX_BODY} characters."));
            return;
        }
        composer.sending = true;
        composer.error = None;

        let (parent, quoted) = match &composer.mode {
            ComposeMode::New => (None, None),
            ComposeMode::Reply(c) => (Some((**c).clone()), None),
            ComposeMode::Quote(c) => (None, Some((**c).clone())),
        };

        // The signed API takes no images yet, so those commits go through the GitHub SSH key, as they do signed out.
        if !signed || !composer.images.is_empty() {
            self.busy = Some("Finding your SSH key…".into());
            self.cli_job = Some(CliJob {
                reply_to: parent.map(|p| p.hash()),
                body,
                images: composer.images.clone(),
            });
            return;
        }

        self.busy = Some("Committing…".into());
        let api = self.api.clone();
        self.spawn(async move {
            let result = api
                .create_commit(
                    &body,
                    parent.as_ref().map(|p| p.id.as_str()),
                    quoted.as_ref().map(|q| q.id.as_str()),
                )
                .await
                .map_err(|e| e.to_string());
            Msg::Posted(result)
        });
    }

    /// What /api/cli/commits answered, once the main loop ran the job.
    pub fn on_cli_commit(&mut self, result: anyhow::Result<String>) {
        self.busy = None;
        match result {
            Ok(message) => {
                let mode = self.composer.as_ref().map(|c| c.mode.clone());
                self.composer = None;
                self.draft = None;
                config::save_draft(None);
                self.draft_images.clear();
                let short = message.lines().next().unwrap_or("Committed.").to_string();
                self.toast(short);
                if let Some(ComposeMode::Reply(parent)) = mode {
                    self.feeds.remove(&Screen::Thread(parent.id.clone()));
                }
                // The new commit shows up in the user's own commits, once it's approved.
                if let Some(username) = self.username() {
                    self.feeds.remove(&Screen::Profile(username, Tab::Commits));
                }
            }
            Err(e) => {
                // Without a key on GitHub, say why one was needed, and what to do instead.
                let message = match (e.downcast_ref::<crate::cli_commit::NoGitHubKey>(), self.signed()) {
                    (Some(_), true) => "Images are signed with the SSH key on your GitHub account, since the app's API can't \
                        take them yet, and no key on this machine is on it. Add one at github.com/settings/keys, or \
                        ⌃X the image to commit without it."
                        .to_string(),
                    (Some(_), false) => format!("{e} Signing in (g s) lets you commit without a key on GitHub, except for images."),
                    (None, _) => e.to_string(),
                };
                if let Some(composer) = self.composer.as_mut() {
                    composer.sending = false;
                    composer.error = Some(message);
                }
            }
        }
    }

    fn delete(&mut self, commit: Commit) {
        let api = self.api.clone();
        self.spawn(async move {
            match api.delete_commit(&commit.id).await {
                Ok(()) => Msg::CommitDeleted(commit.id),
                Err(e) => Msg::Toast(e.to_string(), true),
            }
        });
    }

    // Signing in ------------------------------------------------------------------------------------------------

    pub fn start_sign_in(&mut self) {
        let key = match DeviceKey::load_or_create(&config::key_path()) {
            Ok(key) => Arc::new(key),
            Err(e) => return self.error(e.to_string()),
        };
        let authorization = Authorization::new(&self.api.base, &key.fingerprint());
        // The previous attempt's handler goes first, so this one isn't removed along with it.
        self.sign_in = None;
        let handler = callback::install();
        let opened = system::open(&authorization.url);
        self.sign_in = Some(SignIn {
            key,
            authorization,
            redeeming: false,
            handler,
        });
        self.input = Some(Input {
            kind: InputKind::SignInCode,
            text: Vec::new(),
            cursor: 0,
            error: None,
        });
        if !opened {
            self.toast("Open the sign-in link in your browser (y copies it).");
        }
    }

    fn redeem(&mut self, pasted: String) {
        let Some(sign_in) = self.sign_in.as_mut() else { return };
        let code = match sign_in.authorization.code_from(&pasted) {
            Ok(code) => code,
            Err(e) => {
                if let Some(input) = self.input.as_mut() {
                    input.error = Some(e.to_string());
                }
                return;
            }
        };
        sign_in.redeeming = true;
        let key = sign_in.key.clone();
        let verifier = sign_in.authorization.verifier.clone();
        let api = self.api.clone();
        self.busy = Some("Signing in…".into());
        self.spawn(async move { Msg::SignedIn(api.redeem(&key, &code, &verifier).await.map_err(|e| e.to_string())) });
    }

    fn on_signed_in(&mut self, result: Result<CurrentUser, String>) {
        self.busy = None;
        match result {
            Err(e) => {
                if let Some(sign_in) = self.sign_in.as_mut() {
                    sign_in.redeeming = false;
                }
                let input = self.input.get_or_insert(Input {
                    kind: InputKind::SignInCode,
                    text: Vec::new(),
                    cursor: 0,
                    error: None,
                });
                input.error = Some(e);
            }
            Ok(me) => {
                let key = self.sign_in.take().map(|s| s.key);
                self.input = None;
                self.api = self.api.with_key(key);
                self.config.signed_in = true;
                self.config.username = Some(me.user.username.clone());
                let _ = self.config.save();
                self.toast(format!("Signed in as @{}. Welcome to /dev/null.", me.user.username));
                self.me = Some(me);
                self.feeds.clear();
                self.stack.clear();
                self.navigate(Screen::Log(Log::Latest));
            }
        }
    }

    fn sign_out_locally(&mut self) {
        let _ = std::fs::remove_file(config::key_path());
        let _ = std::fs::remove_file(config::key_path().with_extension("pub"));
        self.api = self.api.with_key(None);
        self.config.signed_in = false;
        let _ = self.config.save();
        self.me = None;
        self.feeds.clear();
        let screen = self.screen().clone();
        self.stack.clear();
        self.navigate(if screen == Screen::Settings {
            Screen::Settings
        } else {
            Screen::Log(Log::Following)
        });
    }

    // Input -----------------------------------------------------------------------------------------------------

    pub fn on_paste(&mut self, text: String) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.input.is_none() && self.composer.is_none() && self.remotes_paste(&text) {
            return;
        }
        if let Some(input) = self.input.as_mut() {
            for c in text.chars().filter(|c| *c != '\n') {
                input.text.insert(input.cursor, c);
                input.cursor += 1;
            }
        } else if let Some(composer) = self.composer.as_mut() {
            // A dropped (or pasted) image file arrives as its path: attach it rather than type it.
            if let Some(paths) = attach::pasted_paths(&text) {
                for path in paths {
                    attach_to(composer, Attachment::from_path(&path));
                }
                return;
            }
            for c in text.chars() {
                composer.text.insert(composer.cursor, c);
                composer.cursor += 1;
            }
        }
    }

    pub fn on_mouse(&mut self, event: MouseEvent) {
        if self.composer.is_some() || self.input.is_some() || self.help {
            if self.help {
                match event.kind {
                    MouseEventKind::ScrollDown => self.help_scroll += 1,
                    MouseEventKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(1),
                    _ => {}
                }
            }
            return;
        }
        match event.kind {
            MouseEventKind::ScrollDown => self.move_by(1),
            MouseEventKind::ScrollUp => self.move_by(-1),
            MouseEventKind::Down(_) => {
                let area = self.feed_area;
                if event.column < area.x || event.column >= area.x + area.width || event.row < area.y || event.row >= area.y + area.height {
                    return;
                }
                let Some(feed) = self.feed_mut() else { return };
                let line = feed.scroll + (event.row - area.y) as usize;
                if let Some(index) = feed.offsets.iter().position(|(start, end)| line >= *start && line < *end) {
                    if feed.selected == index {
                        self.open_selected();
                    } else {
                        feed.selected = index;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            if self.composer.is_some() {
                self.close_composer();
            } else if self.input.is_some() {
                self.input = None;
            } else {
                self.quit = true;
            }
            return;
        }

        if self.input.is_some() {
            return self.on_input_key(key);
        }
        if self.composer.is_some() {
            return self.on_composer_key(key);
        }
        if let Some(confirm) = self.confirm.take() {
            if let Confirm::Report(commit) = confirm {
                if let KeyCode::Char(n @ '1'..='4') = key.code {
                    self.report(commit, REPORT_REASONS[(n as u8 - b'1') as usize].0);
                }
                return;
            }
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Enter) {
                match confirm {
                    Confirm::Report(_) => {}
                    Confirm::Mute(username, on) => self.mute(username, on),
                    Confirm::Block(username, on) => self.block(username, on),
                    Confirm::Delete(commit) => self.delete(commit),
                    Confirm::SignOut => {
                        self.sign_out_locally();
                        self.toast("Signed out. Revoke the device under Settings → Devices on the web, too.");
                    }
                }
            }
            return;
        }
        if self.achievements {
            self.achievements = false;
            return;
        }
        if let Some(viewer) = self.viewer.as_mut() {
            let n = viewer.images.len();
            match key.code {
                KeyCode::Right | KeyCode::Char('l' | 'j' | ' ') => viewer.index = (viewer.index + 1) % n,
                KeyCode::Left | KeyCode::Char('h' | 'k') => viewer.index = (viewer.index + n - 1) % n,
                KeyCode::Char('w') => {
                    if let Some(url) = viewer.images[viewer.index].url.clone() {
                        system::open(&url);
                    }
                }
                _ => self.viewer = None,
            }
            return;
        }
        if self.help {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => self.help_scroll += 1,
                KeyCode::Char('k') | KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                _ => {
                    self.help = false;
                    self.help_scroll = 0;
                }
            }
            return;
        }

        if self.pending_g {
            self.pending_g = false;
            let username = self.username();
            match key.code {
                KeyCode::Char('l') => self.navigate(Screen::Log(if self.log == Log::Trending { Log::Latest } else { self.log })),
                KeyCode::Char('e') => self.navigate(Screen::Log(Log::Trending)),
                KeyCode::Char('f') => self.navigate(Screen::Log(Log::Following)),
                KeyCode::Char('p') => match username {
                    Some(u) => self.navigate(Screen::Profile(u, Tab::Commits)),
                    None => {
                        self.navigate(Screen::Settings);
                        self.toast("Set your username first.");
                    }
                },
                KeyCode::Char('s') => self.navigate(Screen::Settings),
                KeyCode::Char('n') => self.navigate(Screen::Notifications(Filter::All)),
                KeyCode::Char('b') => self.navigate(Screen::Starred),
                KeyCode::Char('r') => self.navigate(Screen::Remotes),
                KeyCode::Char('g') => {
                    if let Some(feed) = self.feed_mut() {
                        feed.selected = 0;
                    }
                }
                _ => {}
            }
            return;
        }

        if self.remotes_key(key) {
            return;
        }

        if *self.screen() == Screen::Settings && self.on_settings_key(key) {
            return;
        }

        match key.code {
            KeyCode::Char('k') if ctrl => self.open_input(InputKind::Go),
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::Char('d') if ctrl => self.move_by(5),
            KeyCode::Char('u') if ctrl => self.move_by(-5),
            KeyCode::PageDown => self.move_by(5),
            KeyCode::PageUp => self.move_by(-5),
            KeyCode::Char('G') | KeyCode::End => {
                if let Some(feed) = self.feed_mut() {
                    feed.selected = feed.items.len().saturating_sub(1);
                }
                self.maybe_load_more();
            }
            KeyCode::Home => {
                if let Some(feed) = self.feed_mut() {
                    feed.selected = 0;
                }
            }
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Enter => self.open_selected(),
            KeyCode::Char('a') => self.open_author(),
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('h') | KeyCode::Left => self.back(),
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('/') => self.open_input(InputKind::Go),
            KeyCode::Char('c') => self.compose(ComposeMode::New),
            KeyCode::Char('r') => {
                if let Some(commit) = self.selected_commit() {
                    self.compose(ComposeMode::Reply(Box::new(commit)));
                }
            }
            KeyCode::Char('q') => {
                if let Some(commit) = self.selected_commit() {
                    self.compose(ComposeMode::Quote(Box::new(commit)));
                }
            }
            KeyCode::Char('o') => self.toggle_replies(),
            KeyCode::Char('y') => self.copy_link(),
            KeyCode::Char('w') => self.open_in_browser(),
            KeyCode::Char('s') => self.star(),
            // b saved a commit, and saved commits became stars, so it stars.
            KeyCode::Char('b') => self.star(),
            KeyCode::Char('p') => self.pastel(),
            KeyCode::Char('f') => self.follow(),
            KeyCode::Char('m') => self.ask_mute_or_block(false),
            KeyCode::Char('v') => self.view_images(),
            KeyCode::Char('A') => {
                if self
                    .feed()
                    .and_then(|f| f.profile.as_ref())
                    .is_some_and(|p| p.achievements.is_some())
                {
                    self.achievements = true;
                } else if matches!(self.screen(), Screen::Profile(..)) {
                    self.toast("Achievements show up when you're signed in.");
                }
            }
            KeyCode::Char('B') => self.ask_mute_or_block(true),
            KeyCode::Char('!') => {
                if self.needs_sign_in("report commits") {
                    return;
                }
                if let Some(commit) = self.selected_commit() {
                    if commit.author.as_ref().map(|a| a.username.clone()) == self.username() {
                        self.error("That's your own commit. D deletes it.");
                    } else {
                        self.confirm = Some(Confirm::Report(commit));
                    }
                }
            }
            KeyCode::Char('R') => self.refresh(),
            KeyCode::Char('D') => {
                if let Some(commit) = self.selected_commit() {
                    let mine = commit.author.as_ref().map(|a| a.username.clone()) == self.username();
                    if mine && self.signed() {
                        self.confirm = Some(Confirm::Delete(commit));
                    } else if mine {
                        self.error("Sign in to delete commits: g s");
                    }
                }
            }
            KeyCode::Tab | KeyCode::Char(']') | KeyCode::Char('l') | KeyCode::Right => self.cycle_tab(1),
            KeyCode::BackTab | KeyCode::Char('[') => self.cycle_tab(-1),
            KeyCode::Char(n @ '1'..='3') => {
                if matches!(self.screen(), Screen::Log(_)) {
                    self.navigate(Screen::Log(Log::ALL[(n as u8 - b'1') as usize]));
                }
            }
            KeyCode::Char('Q') => self.quit = true,
            _ => {}
        }
    }

    fn cycle_tab(&mut self, delta: isize) {
        match self.screen().clone() {
            Screen::Log(log) => {
                let index = Log::ALL.iter().position(|l| *l == log).unwrap_or(0) as isize;
                let next = Log::ALL[(index + delta).rem_euclid(3) as usize];
                self.navigate(Screen::Log(next));
            }
            Screen::Notifications(filter) => {
                let index = Filter::ALL.iter().position(|f| *f == filter).unwrap_or(0) as isize;
                self.stack.pop();
                self.navigate(Screen::Notifications(Filter::ALL[(index + delta).rem_euclid(7) as usize]));
            }
            Screen::Profile(username, tab) => {
                let index = Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0) as isize;
                self.stack.pop();
                self.navigate(Screen::Profile(username, Tab::ALL[(index + delta).rem_euclid(4) as usize]));
            }
            _ => {}
        }
    }

    /// Loads the theme from Settings (or DEVNULL_THEME) again, so a theme file you're editing shows its changes.
    pub fn apply_theme(&mut self) {
        let id = match self.config.theme().as_str() {
            "auto" if self.detected_light == Some(true) => "light".to_string(),
            "auto" => "dark".to_string(),
            id => id.to_string(),
        };
        match crate::theme::load(&id) {
            Ok(palette) => crate::theme::set(palette),
            Err(e) => {
                crate::theme::set(crate::theme::fallback());
                self.error(format!("{e:#}"));
            }
        }
    }

    fn on_settings_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c @ ('t' | 'T')) => {
                // auto, then every theme, built-in and yours. T goes back.
                let mut ids = vec!["auto".to_string()];
                ids.extend(crate::theme::all());
                let at = ids.iter().position(|id| *id == self.config.theme()).unwrap_or(0) as isize;
                let step = if c == 't' { 1 } else { -1 };
                let next = ids[(at + step).rem_euclid(ids.len() as isize) as usize].clone();
                self.config.theme = Some(next);
                let _ = self.config.save();
                self.apply_theme();
                if std::env::var("DEVNULL_THEME").is_ok_and(|t| !t.is_empty()) {
                    self.toast("DEVNULL_THEME is set, so it wins over this setting.");
                }
            }
            KeyCode::Char('s') if !self.signed() => self.start_sign_in(),
            KeyCode::Char('v') if self.sign_in.is_some() => {
                self.input = Some(Input {
                    kind: InputKind::SignInCode,
                    text: Vec::new(),
                    cursor: 0,
                    error: None,
                });
            }
            KeyCode::Char('u') if !self.signed() => self.open_input(InputKind::Username),
            KeyCode::Char('L') if self.signed() => self.confirm = Some(Confirm::SignOut),
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'l') if self.signed() && self.me.is_some() => {
                let current = self.lane_pick.unwrap_or_else(|| self.current_lane_index());
                let delta: isize = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) {
                    -1
                } else {
                    1
                };
                self.lane_pick = Some((current as isize + delta).rem_euclid(7) as usize);
            }
            KeyCode::Enter if self.signed() && self.lane_pick.is_some() => {
                let pick = self.lane_pick.unwrap_or(6);
                let lane = (pick < 6).then_some(pick as u8);
                self.busy = Some("Saving your color…".into());
                let api = self.api.clone();
                self.spawn(async move {
                    Msg::MeUpdated(
                        api.set_lane(lane, None).await.map_err(|e| e.to_string()),
                        "Your color is saved.".into(),
                    )
                });
            }
            KeyCode::Char('#') if self.signed() => self.open_input(InputKind::LaneColor),
            KeyCode::Char('P') if self.signed() => {
                let on = !self.me.as_ref().is_some_and(|m| m.pasteis_mode);
                let api = self.api.clone();
                self.spawn(async move {
                    let done = if on {
                        "Pastéis mode is on: p gives a commit a pastel de nata."
                    } else {
                        "Pastéis mode is off."
                    };
                    Msg::MeUpdated(api.set_pasteis_mode(on).await.map_err(|e| e.to_string()), done.into())
                });
            }
            KeyCode::Char('G') if self.signed() => {
                self.busy = Some("Syncing from GitHub…".into());
                let api = self.api.clone();
                self.spawn(async move {
                    Msg::MeUpdated(
                        api.sync_github().await.map_err(|e| e.to_string()),
                        "Synced your profile from GitHub.".into(),
                    )
                });
            }
            KeyCode::Esc if self.lane_pick.is_some() => self.lane_pick = None,
            KeyCode::Enter if !self.signed() && self.config.username.is_some() => self.navigate(Screen::Log(Log::Following)),
            KeyCode::Char('y') | KeyCode::Char('w') => return false,
            KeyCode::Char('j' | 'k') | KeyCode::Down | KeyCode::Up | KeyCode::Enter => {}
            _ => return false,
        }
        true
    }

    /// Where the signed-in user's lane sits in the picker: 0 to 5, or 6 for Automatic and custom colors.
    pub fn current_lane_index(&self) -> usize {
        match self.me.as_ref().and_then(|m| m.preferred_lane.clone()) {
            Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(6).min(6) as usize,
            _ => 6,
        }
    }

    /// The user's own commits take their new color, wherever they're shown.
    fn recolor(&mut self, me: &User) {
        let lane = me.lane.clone();
        let fix = |user: &mut User| {
            if user.username == me.username {
                user.lane = lane.clone();
            }
        };
        let fix_commit = |c: &mut Commit| {
            if let Some(author) = c.author.as_mut() {
                fix(author);
            }
            if let Some(q) = c.quoted_commit.as_mut()
                && let Some(author) = q.author.as_mut()
            {
                fix(author);
            }
        };
        for feed in self.feeds.values_mut() {
            for item in feed.items.iter_mut() {
                match item {
                    Item::Commit(c, _) => fix_commit(c),
                    Item::Notification(n) => {
                        if let Some(c) = n.commit.as_mut() {
                            fix_commit(c)
                        }
                    }
                    Item::User(u) => fix(u),
                }
            }
            if let Some(profile) = feed.profile.as_mut() {
                fix(&mut profile.data);
            }
        }
    }

    fn open_input(&mut self, kind: InputKind) {
        let text: Vec<char> = match kind {
            InputKind::Username => self.config.username.clone().unwrap_or_default().chars().collect(),
            InputKind::LaneColor => match self.me.as_ref().and_then(|m| m.preferred_lane.clone()) {
                Some(serde_json::Value::String(color)) => color.chars().collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let cursor = text.len();
        self.input = Some(Input {
            kind,
            text,
            cursor,
            error: None,
        });
    }

    fn on_input_key(&mut self, key: KeyEvent) {
        let Some(input) = self.input.as_mut() else { return };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.input = None;
            }
            KeyCode::Enter => {
                let text: String = input.text.iter().collect::<String>().trim().to_string();
                let kind = input.kind;
                match kind {
                    InputKind::Go => {
                        self.input = None;
                        self.go_to(&text);
                    }
                    InputKind::Username => {
                        let username = text.trim_start_matches('@').to_string();
                        let valid = !username.is_empty() && username.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
                        if !valid {
                            input.error = Some("That isn't a GitHub username.".into());
                            return;
                        }
                        self.input = None;
                        self.config.username = Some(username.clone());
                        let _ = self.config.save();
                        self.feeds.remove(&Screen::Log(Log::Following));
                        self.toast(format!("Hi @{username}. Your Following log is built from who you follow."));
                        self.navigate(Screen::Log(Log::Following));
                    }
                    InputKind::LaneColor => {
                        let color = text.trim_start_matches('#').to_lowercase();
                        if color.len() != 6 || !color.chars().all(|c| c.is_ascii_hexdigit()) {
                            input.error = Some("A color is six hex digits, like #3bc9db.".into());
                            return;
                        }
                        self.input = None;
                        self.busy = Some("Saving your color…".into());
                        let api = self.api.clone();
                        self.spawn(async move {
                            let result = api.set_lane(None, Some(&format!("#{color}"))).await.map_err(|e| e.to_string());
                            Msg::MeUpdated(result, format!("Your color is #{color} now."))
                        });
                    }
                    InputKind::SignInCode => {
                        if self.sign_in.as_ref().is_some_and(|s| !s.redeeming) {
                            self.redeem(text);
                        }
                    }
                }
            }
            KeyCode::Char('u') if ctrl => {
                input.text.drain(..input.cursor);
                input.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let mut start = input.cursor;
                while start > 0 && input.text[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && input.text[start - 1] != ' ' {
                    start -= 1;
                }
                input.text.drain(start..input.cursor);
                input.cursor = start;
            }
            KeyCode::Char(c) if !ctrl || key.modifiers.contains(KeyModifiers::ALT) => {
                input.text.insert(input.cursor, c);
                input.cursor += 1;
                input.error = None;
            }
            KeyCode::Backspace if input.cursor > 0 => {
                input.cursor -= 1;
                input.text.remove(input.cursor);
            }
            KeyCode::Delete if input.cursor < input.text.len() => {
                input.text.remove(input.cursor);
            }
            KeyCode::Left => input.cursor = input.cursor.saturating_sub(1),
            KeyCode::Right => input.cursor = (input.cursor + 1).min(input.text.len()),
            KeyCode::Home => input.cursor = 0,
            KeyCode::End => input.cursor = input.text.len(),
            _ => {}
        }
    }

    /// `/`: a username, a profile or commit link, or a commit id.
    fn go_to(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        let is_uuid = |s: &str| s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-');

        let path = query
            .split("://")
            .nth(1)
            .map(|rest| rest.split_once('/').map(|(_, p)| p).unwrap_or(""))
            .unwrap_or(query);
        let parts: Vec<&str> = path.split(['/', '?', '#']).filter(|p| !p.is_empty()).collect();

        if let Some(id) = parts.iter().find(|p| is_uuid(p)) {
            return self.navigate(Screen::Thread(id.to_string()));
        }
        let username = parts.first().map(|p| p.trim_start_matches('@')).unwrap_or("");
        if !username.is_empty() && username.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return self.navigate(Screen::Profile(username.to_string(), Tab::Commits));
        }
        self.error("Try @username, a profile link, or a commit link.");
    }

    fn close_composer(&mut self) {
        if let Some(composer) = self.composer.take()
            && matches!(composer.mode, ComposeMode::New)
            && (!composer.body().is_empty() || !composer.images.is_empty())
        {
            config::save_draft(Some(&composer.text.iter().collect::<String>()));
            self.draft = Some(composer.text);
            self.draft_images = composer.images;
            self.toast("Draft kept. c brings it back.");
        }
        self.busy = None;
    }

    fn on_composer_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let sup = key.modifiers.contains(KeyModifiers::SUPER);

        match key.code {
            KeyCode::Esc => return self.close_composer(),
            KeyCode::Enter if ctrl || alt || sup => return self.submit(),
            KeyCode::Char('s') if ctrl => return self.submit(),
            KeyCode::Char('p' | 'P') if ctrl => {
                if let Some(c) = self.composer.as_mut() {
                    c.preview = !c.preview;
                }
                return;
            }
            KeyCode::Char('v') if ctrl && !alt => {
                if let Some(c) = self.composer.as_mut().filter(|c| !c.sending) {
                    attach_to(c, attach::from_clipboard());
                }
                return;
            }
            KeyCode::Char('x') if ctrl && !alt => {
                if let Some(c) = self.composer.as_mut().filter(|c| !c.sending) {
                    c.images.pop();
                    c.error = None;
                }
                return;
            }
            _ => {}
        }

        // Completing an @mention: Tab takes the picked one, the arrows pick.
        if let Some((at, prefix)) = self
            .composer
            .as_ref()
            .filter(|c| !c.sending && !c.preview)
            .and_then(|c| c.mention())
        {
            let candidates = self.mention_candidates(&prefix);
            if !candidates.is_empty() {
                let c = self.composer.as_mut().expect("composer");
                match key.code {
                    KeyCode::Tab => {
                        let pick = &candidates[c.mention_index.min(candidates.len() - 1)];
                        let completion: Vec<char> = format!("{} ", pick.username).chars().collect();
                        c.text.splice(at + 1..c.cursor, completion.iter().copied());
                        c.cursor = at + 1 + completion.len();
                        c.mention_index = 0;
                        return;
                    }
                    KeyCode::Down => {
                        c.mention_index = (c.mention_index + 1) % candidates.len();
                        return;
                    }
                    KeyCode::Up => {
                        c.mention_index = (c.mention_index + candidates.len() - 1) % candidates.len();
                        return;
                    }
                    _ => {}
                }
            }
        }

        let Some(c) = self.composer.as_mut() else { return };
        if c.sending || c.preview {
            return;
        }
        c.error = None;
        if matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete) {
            c.mention_index = 0;
        }
        match key.code {
            KeyCode::Enter => {
                c.text.insert(c.cursor, '\n');
                c.cursor += 1;
            }
            KeyCode::Char('u') if ctrl => {
                let start = c.text[..c.cursor].iter().rposition(|ch| *ch == '\n').map(|i| i + 1).unwrap_or(0);
                c.text.drain(start..c.cursor);
                c.cursor = start;
            }
            KeyCode::Char('w') if ctrl => {
                let mut start = c.cursor;
                while start > 0 && c.text[start - 1].is_whitespace() {
                    start -= 1;
                }
                while start > 0 && !c.text[start - 1].is_whitespace() {
                    start -= 1;
                }
                c.text.drain(start..c.cursor);
                c.cursor = start;
            }
            // AltGr arrives as ⌃⌥ on some layouts, and types characters like @.
            KeyCode::Char(ch) if !ctrl || alt => {
                c.text.insert(c.cursor, ch);
                c.cursor += 1;
            }
            KeyCode::Tab => {
                c.text.insert(c.cursor, ' ');
                c.text.insert(c.cursor, ' ');
                c.cursor += 2;
            }
            KeyCode::Backspace if c.cursor > 0 => {
                c.cursor -= 1;
                c.text.remove(c.cursor);
            }
            KeyCode::Delete if c.cursor < c.text.len() => {
                c.text.remove(c.cursor);
            }
            KeyCode::Left => c.cursor = c.cursor.saturating_sub(1),
            KeyCode::Right => c.cursor = (c.cursor + 1).min(c.text.len()),
            KeyCode::Home => c.cursor = c.text[..c.cursor].iter().rposition(|ch| *ch == '\n').map(|i| i + 1).unwrap_or(0),
            KeyCode::End => {
                c.cursor += c.text[c.cursor..]
                    .iter()
                    .position(|ch| *ch == '\n')
                    .unwrap_or(c.text.len() - c.cursor)
            }
            KeyCode::Up | KeyCode::Down => {
                let line_start = c.text[..c.cursor].iter().rposition(|ch| *ch == '\n').map(|i| i + 1).unwrap_or(0);
                let column = c.cursor - line_start;
                if key.code == KeyCode::Up {
                    if line_start == 0 {
                        c.cursor = 0;
                    } else {
                        let prev_start = c.text[..line_start - 1]
                            .iter()
                            .rposition(|ch| *ch == '\n')
                            .map(|i| i + 1)
                            .unwrap_or(0);
                        c.cursor = (prev_start + column).min(line_start - 1);
                    }
                } else if let Some(next) = c.text[c.cursor..].iter().position(|ch| *ch == '\n') {
                    let next_start = c.cursor + next + 1;
                    let next_end = next_start
                        + c.text[next_start..]
                            .iter()
                            .position(|ch| *ch == '\n')
                            .unwrap_or(c.text.len() - next_start);
                    c.cursor = (next_start + column).min(next_end);
                } else {
                    c.cursor = c.text.len();
                }
            }
            _ => {}
        }
    }
}

fn attach_to(composer: &mut Composer, attachment: anyhow::Result<Attachment>) {
    match attachment {
        Ok(_) if composer.images.len() >= attach::MAX_IMAGES => {
            composer.error = Some(format!("A commit can carry up to {} images.", attach::MAX_IMAGES));
        }
        Ok(image) => {
            composer.images.push(image);
            composer.error = None;
        }
        Err(e) => composer.error = Some(e.to_string()),
    }
}

fn apply(feed: &mut Feed, loaded: Loaded, more: bool) {
    let selected_id = feed.items.get(feed.selected).and_then(|i| i.commit()).map(|c| c.id.clone());
    match loaded {
        Loaded::Commits(commits, cursor) => {
            let items = commits.into_iter().map(|c| Item::Commit(Box::new(c), Role::Normal));
            if more {
                feed.items.extend(items);
            } else {
                feed.items = items.collect();
                feed.new_count = 0;
            }
            feed.cursor = cursor;
        }
        Loaded::Notifications(notifications, cursor) => {
            let items = notifications.into_iter().map(|n| Item::Notification(Box::new(n)));
            if more {
                feed.items.extend(items);
            } else {
                feed.items = items.collect();
            }
            feed.cursor = cursor;
        }
        Loaded::Thread {
            ancestors,
            focus,
            replies,
            cursor,
            notice,
        } => {
            if more {
                feed.items
                    .extend(replies.into_iter().map(|c| Item::Commit(Box::new(c), Role::Reply)));
            } else {
                let focus_index = ancestors.len();
                feed.items = ancestors.into_iter().map(|c| Item::Commit(Box::new(c), Role::Ancestor)).collect();
                feed.items.push(Item::Commit(Box::new(focus), Role::Focus));
                feed.items
                    .extend(replies.into_iter().map(|c| Item::Commit(Box::new(c), Role::Reply)));
                feed.selected = focus_index;
                feed.notice = notice;
                feed.cursor = cursor;
                return;
            }
            feed.cursor = cursor;
        }
        Loaded::Notice(notice) => {
            feed.items.clear();
            feed.cursor = None;
            feed.notice = Some(notice);
            return;
        }
        Loaded::Profile(profile, items, cursor) => {
            feed.profile = Some(profile);
            if more {
                feed.items.extend(items);
            } else {
                feed.items = items;
            }
            feed.cursor = cursor;
        }
        Loaded::People(users, cursor) => {
            feed.items.extend(users.into_iter().map(|u| Item::User(Box::new(u))));
            feed.cursor = cursor;
        }
    }

    if !more {
        // Keep the selection on the same commit across a refresh.
        feed.selected = selected_id
            .and_then(|id| feed.items.iter().position(|i| i.commit().is_some_and(|c| c.id == id)))
            .unwrap_or(0);
        if feed.selected == 0 {
            feed.scroll = 0;
        }
    }
}

async fn fetch(api: Api, screen: Screen, cursor: Option<String>, signed: bool, guest: Option<String>) -> Result<Loaded, String> {
    let cursor = cursor.as_deref();
    let err = |e: anyhow::Error| e.to_string();
    match screen {
        Screen::Log(Log::Following) if !signed => {
            let Some(username) = guest else {
                return Ok(Loaded::Notice("Set your GitHub username in Settings (g s) to build your Following log from who you follow, or sign in for the real thing.".into()));
            };
            guest_following(&api, &username).await.map_err(err)
        }
        Screen::Log(_) | Screen::Notifications(_) | Screen::Starred if !signed => Ok(Loaded::Notice(format!(
            "{} needs you to sign in: g s. Until then, Following (1) is built from public commits.",
            screen.title()
        ))),
        Screen::Log(log) => {
            let page = api.log(log, cursor).await.map_err(err)?;
            let mut commits = page.data;
            if commits.is_empty() && cursor.is_none() {
                commits = page.suggestions;
            }
            Ok(Loaded::Commits(commits, page.meta.next_cursor))
        }
        Screen::Notifications(filter) => {
            let page = api.notifications(filter.query(), cursor).await.map_err(err)?;
            Ok(Loaded::Notifications(page.data, page.meta.next_cursor))
        }
        Screen::Starred => {
            let page = api.starred(cursor).await.map_err(err)?;
            Ok(Loaded::Commits(page.data, page.meta.next_cursor))
        }
        Screen::Profile(username, tab) if tab.is_people() => {
            let people = async {
                if tab == Tab::Followers {
                    api.followers(&username, cursor).await
                } else {
                    api.following(&username, cursor).await
                }
            };
            if cursor.is_some() {
                let page = people.await.map_err(err)?;
                return Ok(Loaded::People(page.data, page.meta.next_cursor));
            }
            let (profile, page) = tokio::join!(api.user(&username), people);
            let (profile, page) = (profile.map_err(err)?, page.map_err(err)?);
            let items = page.data.into_iter().map(|u| Item::User(Box::new(u))).collect();
            Ok(Loaded::Profile(profile, items, page.meta.next_cursor))
        }
        Screen::Profile(username, tab) => {
            let replies = tab == Tab::Replies;
            let commits = api.user_commits(&username, replies, cursor);
            if cursor.is_some() {
                let page = commits.await.map_err(err)?;
                return Ok(Loaded::Commits(page.data, page.meta.next_cursor));
            }
            let (profile, page) = tokio::join!(api.user(&username), commits);
            let profile = profile.map_err(err)?;
            let page = page.map_err(err)?;
            let mut items = page.data;
            if let Some(pinned) = profile.pinned_commit.clone().filter(|_| !replies) {
                items.retain(|c| c.id != pinned.id);
                items.insert(0, pinned);
            }
            let items = items.into_iter().map(|c| Item::Commit(Box::new(c), Role::Normal)).collect();
            Ok(Loaded::Profile(profile, items, page.meta.next_cursor))
        }
        Screen::Thread(id) => {
            if cursor.is_some() {
                let page = api.replies(&id, cursor).await.map_err(err)?;
                return Ok(Loaded::Thread {
                    ancestors: vec![],
                    focus: Commit::default(),
                    replies: page.data,
                    cursor: page.meta.next_cursor,
                    notice: None,
                });
            }
            let page = api.commit(&id).await.map_err(err)?;
            if signed {
                let replies = api.replies(&id, None).await.map_err(err)?;
                return Ok(Loaded::Thread {
                    ancestors: page.ancestors,
                    focus: page.data,
                    replies: replies.data,
                    cursor: replies.meta.next_cursor,
                    notice: None,
                });
            }
            // The public API has no replies, but it does say which public commit this one replies to.
            let mut ancestors = Vec::new();
            let mut parent = page.data.parent_id.clone();
            while let Some(parent_id) = parent.take() {
                if ancestors.len() >= 8 {
                    break;
                }
                if let Ok(p) = api.commit(&parent_id).await {
                    parent = p.data.parent_id.clone();
                    ancestors.insert(0, p.data);
                }
            }
            let notice = (page.data.replies_count.unwrap_or(0) > 0).then(|| "Sign in to read the replies (g s).".to_string());
            Ok(Loaded::Thread {
                ancestors,
                focus: page.data,
                replies: vec![],
                cursor: None,
                notice,
            })
        }
        Screen::Settings | Screen::Remotes | Screen::Remote(_) => Err(String::new()),
    }
}

/// Without signing in there is no log, so Following is put together from the public commits of the people the user
/// follows, newest first.
async fn guest_following(api: &Api, username: &str) -> anyhow::Result<Loaded> {
    let following = api.following(username, None).await?;
    let mut names: Vec<String> = following.data.into_iter().map(|u| u.username).collect();
    names.push(username.to_string());

    let pages = futures::future::join_all(names.iter().map(|name| api.user_commits(name, false, None))).await;
    let mut commits: Vec<Commit> = pages.into_iter().filter_map(Result::ok).flat_map(|p| p.data).collect();
    commits.sort_by_key(|c| std::cmp::Reverse(c.created_at));
    commits.truncate(100);
    Ok(Loaded::Commits(commits, None))
}
