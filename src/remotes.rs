//! Remotes, /dev/null's direct messages: the inbox, a conversation and starting one. This is the state and what the
//! keys do; `ui/remotes_view.rs` draws it. The endpoints it uses are listed in docs/api-contract.md.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Msg, Screen, Tab, Viewer};
use crate::auth::random_hex;
use crate::model::*;
use crate::system;

/// What the server accepts in one message.
pub const MAX_MESSAGE: usize = 2000;
/// The quick reactions, on 1 to 6. The server only takes emoji from its own list, which has all of these.
pub const REACTIONS: [&str; 6] = ["\u{2764}\u{fe0f}", "👍", "😂", "🎉", "👀", "🙏"];

const THREAD_EVERY: Duration = Duration::from_secs(4);
const INBOX_EVERY: Duration = Duration::from_secs(12);
const UNREAD_EVERY: Duration = Duration::from_secs(60);
const TYPING_PAUSE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum InboxTab {
    #[default]
    Open,
    Stashed,
}

impl InboxTab {
    pub const ALL: [InboxTab; 2] = [InboxTab::Open, InboxTab::Stashed];

    pub fn title(self) -> &'static str {
        match self {
            InboxTab::Open => "Open",
            InboxTab::Stashed => "Stashed",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            InboxTab::Open => "open",
            InboxTab::Stashed => "stashed",
        }
    }
}

#[derive(Default)]
pub struct Remotes {
    pub tab: InboxTab,
    pub query: String,
    /// Typing in the search field.
    pub searching: bool,
    pub list: Vec<Conversation>,
    pub selected: usize,
    pub scroll: usize,
    pub loading: bool,
    pub loaded: bool,
    pub has_more: bool,
    pub error: Option<String>,
    /// Why there is no inbox, such as Remotes being off.
    pub notice: Option<String>,
    pub unread: u64,
    pub requests: u64,
    pub chats: HashMap<String, Chat>,
    pub picker: Option<Picker>,
    pub checked_at: Option<Instant>,
    pub unread_checked_at: Option<Instant>,
    pub search_edited: Option<Instant>,
}

/// One conversation, as loaded and as being written to.
#[derive(Default)]
pub struct Chat {
    pub partner: Option<User>,
    pub messages: Vec<Message>,
    pub has_older: bool,
    pub loaded: bool,
    pub loading: bool,
    pub polling: bool,
    pub older_loading: bool,
    pub error: Option<String>,
    pub reactions_version: i64,
    /// The newest message the user has pulled, and how far the partner has.
    pub read_through: i64,
    pub partner_read_through: i64,
    pub partner_read_at: Option<chrono::DateTime<chrono::Utc>>,
    pub partner_typing: bool,
    pub is_archived: bool,
    /// What the composer can do: open, request, waiting, restricted or closed.
    pub access: String,
    pub selected: usize,
    /// The first rendered line on screen, kept between frames so the thread only moves when the selection leaves it.
    pub top: usize,
    pub input: Vec<char>,
    pub cursor: usize,
    pub typing: bool,
    pub sending: bool,
    pub reply_to: Option<i64>,
    /// Waiting for 1 to 6 to pick a reaction.
    pub react: bool,
    pub polled_at: Option<Instant>,
}

pub struct Picker {
    pub query: String,
    pub candidates: Vec<Candidate>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// What the candidates on screen were asked for, and when the query last changed.
    pub asked: Option<String>,
    pub edited: Option<Instant>,
}

pub enum RemoteMsg {
    Inbox {
        tab: InboxTab,
        term: String,
        more: bool,
        result: Result<Inbox, String>,
    },
    Opened {
        handle: String,
        result: Result<Thread, String>,
    },
    Polled {
        handle: String,
        result: Result<Thread, String>,
    },
    Older {
        handle: String,
        result: Result<Thread, String>,
    },
    Sent {
        handle: String,
        result: Result<Message, String>,
    },
    Stashed {
        handle: String,
        on: bool,
        result: Result<(), String>,
    },
    Reacted {
        handle: String,
        message: i64,
        result: Result<Vec<Reaction>, String>,
    },
    Candidates {
        term: String,
        result: Result<Vec<Candidate>, String>,
    },
    Pulled,
    Unread(u64),
    Preferences {
        shares: bool,
        result: Result<(), String>,
    },
}

/// A version 4 UUID, which is what the server wants a message's `client_id` to be.
fn uuid() -> String {
    let mut hex = random_hex(16).into_bytes();
    hex[12] = b'4';
    hex[16] = b"89ab"[hex[16] as usize % 4];
    let s = String::from_utf8(hex).unwrap_or_default();
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32])
}

/// Why the composer is closed, for each answer of `access` but open.
pub fn access_notice(access: &str, handle: &str) -> Option<String> {
    match access {
        "request" => Some(format!("Merge @{handle}'s request on the web before replying.")),
        "waiting" => Some(format!("@{handle} hasn't merged your request yet.")),
        "restricted" => Some(format!("@{handle} doesn't follow you, so you can't push to them yet.")),
        "closed" => Some(format!("@{handle} isn't taking remotes.")),
        _ => None,
    }
}

fn failure(e: anyhow::Error) -> String {
    e.to_string()
}

impl Chat {
    pub fn last_id(&self) -> i64 {
        self.messages.last().map_or(0, |m| m.id)
    }

    pub fn selected_message(&self) -> Option<&Message> {
        self.messages.get(self.selected)
    }

    fn follows_latest(&self) -> bool {
        self.selected + 1 >= self.messages.len()
    }

    fn text(&self) -> String {
        self.input.iter().collect()
    }
}

impl Remotes {
    pub fn selected_conversation(&self) -> Option<&Conversation> {
        self.list.get(self.selected)
    }
}

impl App {
    // Entering and leaving ----------------------------------------------------------------------------------------

    /// A Remotes screen was pushed: read what it shows.
    pub fn remotes_opened(&mut self, screen: &Screen) {
        match screen {
            Screen::Remotes => {
                self.remotes.checked_at = Some(Instant::now());
                self.load_inbox(false);
            }
            Screen::Remote(handle) => self.open_chat(handle),
            _ => {}
        }
    }

    /// Back from a conversation, its unread count has changed.
    pub fn remotes_left(&mut self, screen: &Screen) {
        if matches!(screen, Screen::Remote(_)) {
            self.remotes.checked_at = None;
        }
    }

    pub fn remotes_refresh(&mut self) {
        match self.screen().clone() {
            Screen::Remotes => self.load_inbox(false),
            Screen::Remote(handle) => self.open_chat(&handle),
            _ => {}
        }
    }

    fn load_inbox(&mut self, more: bool) {
        let r = &mut self.remotes;
        if more && (r.loading || !r.has_more) {
            return;
        }
        let before = if more { r.list.last().map(|c| c.id.clone()) } else { None };
        r.loading = true;
        r.error = None;
        let (tab, term) = (r.tab, r.query.trim().to_string());
        let api = self.api.clone();
        self.spawn(async move {
            let result = api.remotes(tab.slug(), &term, before.as_deref()).await.map_err(failure);
            Msg::Remote(RemoteMsg::Inbox { tab, term, more, result })
        });
    }

    fn open_chat(&mut self, handle: &str) {
        let partner = self
            .remotes
            .list
            .iter()
            .find_map(|c| c.partner.as_ref().filter(|p| p.username == handle).cloned());
        let chat = self.remotes.chats.entry(handle.to_string()).or_default();
        if chat.partner.is_none() {
            chat.partner = partner;
        }
        chat.loading = true;
        chat.error = None;
        chat.polled_at = Some(Instant::now());
        let (api, handle) = (self.api.clone(), handle.to_string());
        self.spawn(async move {
            let result = api.remote_thread(&handle, None, None, None).await.map_err(failure);
            Msg::Remote(RemoteMsg::Opened { handle, result })
        });
    }

    fn poll_chat(&mut self, handle: &str) {
        let Some(chat) = self.remotes.chats.get_mut(handle) else { return };
        chat.polling = true;
        chat.polled_at = Some(Instant::now());
        let (after, version) = (chat.last_id(), chat.reactions_version);
        let (api, handle) = (self.api.clone(), handle.to_string());
        self.spawn(async move {
            let result = api.remote_thread(&handle, Some(after), None, Some(version)).await.map_err(failure);
            Msg::Remote(RemoteMsg::Polled { handle, result })
        });
    }

    fn load_older(&mut self, handle: &str) {
        let Some(chat) = self.remotes.chats.get_mut(handle) else { return };
        if chat.older_loading || !chat.has_older {
            return;
        }
        chat.older_loading = true;
        let before = chat.messages.first().map(|m| m.id);
        let (api, handle) = (self.api.clone(), handle.to_string());
        self.spawn(async move {
            let result = api.remote_thread(&handle, None, before, None).await.map_err(failure);
            Msg::Remote(RemoteMsg::Older { handle, result })
        });
    }

    /// Pulls the conversation through its newest message from the partner, once, when there is one not pulled yet.
    fn pull_chat(&mut self, handle: &str) {
        let Some(chat) = self.remotes.chats.get_mut(handle) else { return };
        let Some(partner) = chat.partner.as_ref().map(|p| p.id.clone()) else {
            return;
        };
        let newest = chat.messages.iter().rev().find(|m| m.user_id == partner).map_or(0, |m| m.id);
        if newest <= chat.read_through {
            return;
        }
        chat.read_through = newest;
        for conversation in self.remotes.list.iter_mut() {
            if conversation.partner.as_ref().is_some_and(|p| p.username == handle) {
                conversation.ahead = 0;
            }
        }
        let (api, handle) = (self.api.clone(), handle.to_string());
        self.spawn(async move {
            let _ = api.pull_remote(&handle, newest).await;
            Msg::Remote(RemoteMsg::Pulled)
        });
    }

    // Answers -----------------------------------------------------------------------------------------------------

    pub fn on_remote_msg(&mut self, msg: RemoteMsg) {
        match msg {
            RemoteMsg::Inbox { tab, term, more, result } => {
                let off = self.me.as_ref().is_some_and(|me| me.remotes.is_none());
                let r = &mut self.remotes;
                // An answer to a search or a tab that has been left behind.
                if tab != r.tab || term != r.query.trim() {
                    return;
                }
                r.loading = false;
                r.loaded = true;
                match result {
                    Ok(inbox) => {
                        let keep = r.list.get(r.selected).map(|c| c.id.clone());
                        if more {
                            for conversation in inbox.data {
                                if !r.list.iter().any(|c| c.id == conversation.id) {
                                    r.list.push(conversation);
                                }
                            }
                        } else {
                            r.list = inbox.data;
                        }
                        r.selected = keep
                            .and_then(|id| r.list.iter().position(|c| c.id == id))
                            .unwrap_or(r.selected)
                            .min(r.list.len().saturating_sub(1));
                        r.has_more = inbox.has_more;
                        r.unread = inbox.unread_count;
                        r.requests = inbox.requests_count;
                        r.notice = None;
                    }
                    Err(e) if off && e == "Not found." => {
                        r.notice = Some("Remotes are off. Turn them on under Settings → Profile on the web.".into());
                    }
                    Err(e) => r.error = Some(e),
                }
            }
            RemoteMsg::Opened { handle, result } => {
                let Some(chat) = self.remotes.chats.get_mut(&handle) else { return };
                chat.loading = false;
                match result {
                    Ok(thread) => {
                        let keep = chat.selected_message().map(|m| m.id);
                        chat.apply_state(&thread);
                        chat.partner = Some(thread.partner);
                        chat.messages = thread.messages;
                        chat.has_older = thread.has_older;
                        chat.loaded = true;
                        chat.error = None;
                        chat.selected = keep
                            .and_then(|id| chat.messages.iter().position(|m| m.id == id))
                            .unwrap_or(chat.messages.len().saturating_sub(1));
                    }
                    Err(e) => chat.error = Some(e),
                }
                self.pull_chat(&handle);
            }
            RemoteMsg::Polled { handle, result } => {
                let Some(chat) = self.remotes.chats.get_mut(&handle) else { return };
                chat.polling = false;
                // A poll that fails says nothing: the next one tries again.
                let Ok(thread) = result else { return };
                let follow = chat.follows_latest();
                chat.apply_state(&thread);
                for change in thread.reaction_changes {
                    if let Some(message) = chat.messages.iter_mut().find(|m| m.id == change.id) {
                        message.reactions = change.reactions;
                    }
                }
                for message in thread.messages {
                    if !chat.messages.iter().any(|m| m.id == message.id) {
                        chat.messages.push(message);
                    }
                }
                if follow {
                    chat.selected = chat.messages.len().saturating_sub(1);
                }
                self.pull_chat(&handle);
            }
            RemoteMsg::Older { handle, result } => {
                let Some(chat) = self.remotes.chats.get_mut(&handle) else { return };
                chat.older_loading = false;
                match result {
                    Ok(thread) => {
                        let mut older: Vec<Message> = thread
                            .messages
                            .into_iter()
                            .filter(|m| !chat.messages.iter().any(|known| known.id == m.id))
                            .collect();
                        chat.selected += older.len();
                        older.append(&mut chat.messages);
                        chat.messages = older;
                        chat.has_older = thread.has_older;
                    }
                    Err(e) => self.error(e),
                }
            }
            RemoteMsg::Sent { handle, result } => {
                let Some(chat) = self.remotes.chats.get_mut(&handle) else { return };
                chat.sending = false;
                match result {
                    Ok(message) => {
                        if !chat.messages.iter().any(|m| m.id == message.id) {
                            chat.messages.push(message);
                        }
                        chat.input.clear();
                        chat.cursor = 0;
                        chat.reply_to = None;
                        chat.selected = chat.messages.len() - 1;
                        self.remotes.checked_at = None;
                    }
                    Err(e) => self.error(e),
                }
            }
            RemoteMsg::Stashed { handle, on, result } => match result {
                Ok(()) => {
                    self.remotes
                        .list
                        .retain(|c| !c.partner.as_ref().is_some_and(|p| p.username == handle));
                    self.remotes.selected = self.remotes.selected.min(self.remotes.list.len().saturating_sub(1));
                    if let Some(chat) = self.remotes.chats.get_mut(&handle) {
                        chat.is_archived = on;
                    }
                    self.toast(if on {
                        format!("Stashed @{handle}. It pops back into Open when either of you writes.")
                    } else {
                        format!("@{handle} is back in Open.")
                    });
                }
                Err(e) => self.error(e),
            },
            RemoteMsg::Reacted { handle, message, result } => match result {
                Ok(reactions) => {
                    if let Some(m) = self
                        .remotes
                        .chats
                        .get_mut(&handle)
                        .and_then(|chat| chat.messages.iter_mut().find(|m| m.id == message))
                    {
                        m.reactions = reactions;
                    }
                }
                Err(e) => self.error(e),
            },
            RemoteMsg::Candidates { term, result } => {
                let Some(picker) = self.remotes.picker.as_mut() else { return };
                if term != picker.query.trim() {
                    return;
                }
                picker.loading = false;
                match result {
                    Ok(candidates) => {
                        picker.candidates = candidates;
                        picker.selected = 0;
                        picker.error = None;
                    }
                    Err(e) => picker.error = Some(e),
                }
            }
            RemoteMsg::Pulled => {
                self.remotes.checked_at = None;
                self.remotes.unread_checked_at = None;
            }
            RemoteMsg::Unread(count) => self.remotes.unread = count,
            RemoteMsg::Preferences { shares, result } => match result {
                Ok(()) => {
                    self.load_me();
                    self.toast(if shares {
                        "Sharing pulls: people see when you pull their messages, and you see theirs."
                    } else {
                        "Not sharing pulls: nobody sees when you pull their messages, and you don't see theirs."
                    });
                }
                Err(e) => self.error(e),
            },
        }
    }

    // Time --------------------------------------------------------------------------------------------------------

    pub fn remotes_tick(&mut self) {
        if !self.signed() {
            return;
        }
        let now = Instant::now();

        if let Some(at) = self.remotes.search_edited
            && at.elapsed() > TYPING_PAUSE
        {
            self.remotes.search_edited = None;
            self.load_inbox(false);
        }
        if let Some(picker) = self.remotes.picker.as_mut()
            && picker.edited.is_some_and(|at| at.elapsed() > TYPING_PAUSE)
        {
            picker.edited = None;
            self.ask_candidates();
        }

        match self.screen().clone() {
            Screen::Remotes => {
                let r = &mut self.remotes;
                if !r.loading && r.checked_at.is_none_or(|at| at.elapsed() > INBOX_EVERY) {
                    r.checked_at = Some(now);
                    self.load_inbox(false);
                }
            }
            Screen::Remote(handle) => {
                if let Some(chat) = self.remotes.chats.get(&handle)
                    && chat.loaded
                    && !chat.polling
                    && chat.polled_at.is_none_or(|at| at.elapsed() > THREAD_EVERY)
                {
                    self.poll_chat(&handle);
                }
            }
            _ => {}
        }

        // The badge on the sidebar, which the inbox and conversations keep up to date themselves.
        let on_remotes = matches!(self.screen(), Screen::Remotes | Screen::Remote(_));
        if !on_remotes
            && self.me.as_ref().is_some_and(|me| me.remotes.is_some())
            && self.remotes.unread_checked_at.is_none_or(|at| at.elapsed() > UNREAD_EVERY)
        {
            self.remotes.unread_checked_at = Some(now);
            let api = self.api.clone();
            self.spawn(async move {
                match api.me().await.ok().and_then(|me| me.remotes) {
                    Some(remotes) => Msg::Remote(RemoteMsg::Unread(remotes.unread_count)),
                    None => Msg::Remote(RemoteMsg::Pulled),
                }
            });
        }
    }

    // Keys --------------------------------------------------------------------------------------------------------

    /// Whether Remotes took the key. It takes every key while something is being typed.
    pub fn remotes_key(&mut self, key: KeyEvent) -> bool {
        if self.remotes.picker.is_some() {
            self.picker_key(key);
            return true;
        }
        match self.screen().clone() {
            Screen::Remotes => self.inbox_key(key),
            Screen::Remote(handle) => self.chat_key(&handle, key),
            _ => false,
        }
    }

    /// The wheel: moves through the inbox or the conversation.
    pub fn remotes_scroll(&mut self, delta: isize) -> bool {
        match self.screen().clone() {
            Screen::Remotes => self.move_inbox(delta),
            Screen::Remote(handle) => self.move_chat(&handle, delta),
            _ => return false,
        }
        true
    }

    pub fn remotes_paste(&mut self, text: &str) -> bool {
        if let Some(picker) = self.remotes.picker.as_mut() {
            picker.query.extend(text.chars().filter(|c| !c.is_control()));
            picker.edited = Some(Instant::now());
            return true;
        }
        if self.remotes.searching {
            self.remotes.query.extend(text.chars().filter(|c| !c.is_control()));
            self.remotes.search_edited = Some(Instant::now());
            return true;
        }
        if let Screen::Remote(handle) = self.screen().clone()
            && let Some(chat) = self.remotes.chats.get_mut(&handle)
            && chat.typing
        {
            for c in text.chars().filter(|c| *c == '\n' || !c.is_control()) {
                chat.insert(c);
            }
            return true;
        }
        false
    }

    fn inbox_key(&mut self, key: KeyEvent) -> bool {
        if self.remotes.searching {
            let r = &mut self.remotes;
            match key.code {
                KeyCode::Esc => {
                    r.searching = false;
                    if !r.query.is_empty() {
                        r.query.clear();
                        self.load_inbox(false);
                    }
                }
                KeyCode::Enter => {
                    r.searching = false;
                    r.search_edited = None;
                    self.load_inbox(false);
                }
                KeyCode::Backspace => {
                    r.query.pop();
                    r.search_edited = Some(Instant::now());
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    r.query.push(c);
                    r.search_edited = Some(Instant::now());
                }
                _ => {}
            }
            return true;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_inbox(1),
            KeyCode::Char('k') | KeyCode::Up if !ctrl => self.move_inbox(-1),
            KeyCode::Char('d') if ctrl => self.move_inbox(5),
            KeyCode::Char('u') if ctrl => self.move_inbox(-5),
            KeyCode::PageDown => self.move_inbox(5),
            KeyCode::PageUp => self.move_inbox(-5),
            KeyCode::Char('G') | KeyCode::End => self.move_inbox(isize::MAX / 2),
            KeyCode::Home => self.move_inbox(isize::MIN / 2),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.open_conversation(),
            KeyCode::Tab | KeyCode::Char(']') => self.switch_inbox_tab(1),
            KeyCode::BackTab | KeyCode::Char('[') => self.switch_inbox_tab(-1),
            KeyCode::Char('/') => self.remotes.searching = true,
            KeyCode::Char('n') => self.open_picker(),
            KeyCode::Char('x') => {
                if let Some(conversation) = self.remotes.selected_conversation()
                    && let Some(partner) = &conversation.partner
                {
                    let (handle, on) = (partner.username.clone(), !conversation.is_archived);
                    self.stash(handle, on);
                }
            }
            KeyCode::Char('a') => {
                if let Some(partner) = self.remotes.selected_conversation().and_then(|c| c.partner.clone()) {
                    self.navigate(Screen::Profile(partner.username, Tab::Commits));
                }
            }
            KeyCode::Char('S') => self.toggle_sharing(),
            KeyCode::Char('R') => self.load_inbox(false),
            // Esc leaves a search that has been applied before it leaves the screen.
            KeyCode::Esc if !self.remotes.query.is_empty() => {
                self.remotes.query.clear();
                self.load_inbox(false);
            }
            _ => return false,
        }
        true
    }

    fn move_inbox(&mut self, delta: isize) {
        let r = &mut self.remotes;
        if r.list.is_empty() {
            return;
        }
        let last = r.list.len() as isize - 1;
        r.selected = (r.selected as isize).saturating_add(delta).clamp(0, last) as usize;
        if r.selected + 4 >= r.list.len() {
            self.load_inbox(true);
        }
    }

    fn switch_inbox_tab(&mut self, delta: isize) {
        let r = &mut self.remotes;
        let index = InboxTab::ALL.iter().position(|t| *t == r.tab).unwrap_or(0) as isize;
        r.tab = InboxTab::ALL[(index + delta).rem_euclid(InboxTab::ALL.len() as isize) as usize];
        r.list.clear();
        r.selected = 0;
        r.scroll = 0;
        r.loaded = false;
        r.has_more = false;
        self.load_inbox(false);
    }

    fn open_conversation(&mut self) {
        let Some(conversation) = self.remotes.selected_conversation() else {
            return;
        };
        match &conversation.partner {
            Some(partner) => {
                let handle = partner.username.clone();
                self.navigate(Screen::Remote(handle));
            }
            None => self.toast("Orgs aren't in the terminal yet. Open this one on the web."),
        }
    }

    fn stash(&mut self, handle: String, on: bool) {
        let api = self.api.clone();
        self.spawn(async move {
            let result = api.stash_remote(&handle, on).await.map_err(failure);
            Msg::Remote(RemoteMsg::Stashed { handle, on, result })
        });
    }

    fn toggle_sharing(&mut self) {
        let Some(settings) = self.me.as_ref().and_then(|me| me.remotes.clone()) else {
            self.error("Remotes are off. Turn them on under Settings → Profile on the web.");
            return;
        };
        let shares = !settings.pull_receipts;
        let api = self.api.clone();
        self.spawn(async move {
            let result = api.set_remote_preferences(&settings.from, shares).await.map_err(failure);
            Msg::Remote(RemoteMsg::Preferences { shares, result })
        });
    }

    fn chat_key(&mut self, handle: &str, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(chat) = self.remotes.chats.get_mut(handle) else {
            return false;
        };

        if chat.typing {
            self.compose_key(handle, key);
            return true;
        }

        if chat.react {
            chat.react = false;
            if let KeyCode::Char(n @ '1'..='6') = key.code {
                self.react((n as u8 - b'1') as usize, handle);
            }
            return true;
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_chat(handle, 1),
            KeyCode::Char('k') | KeyCode::Up if !ctrl => self.move_chat(handle, -1),
            KeyCode::Char('d') if ctrl => self.move_chat(handle, 5),
            KeyCode::Char('u') if ctrl => self.move_chat(handle, -5),
            KeyCode::PageDown => self.move_chat(handle, 5),
            KeyCode::PageUp => self.move_chat(handle, -5),
            KeyCode::Char('G') | KeyCode::End => self.move_chat(handle, isize::MAX / 2),
            KeyCode::Home => self.move_chat(handle, isize::MIN / 2),
            KeyCode::Enter | KeyCode::Char('i') => self.start_typing(handle),
            KeyCode::Char('r') => {
                if let Some(id) = chat.selected_message().map(|m| m.id) {
                    chat.reply_to = Some(id);
                    self.start_typing(handle);
                }
            }
            KeyCode::Char('e') => {
                if chat.selected_message().is_some() && chat.access == "open" {
                    chat.react = true;
                }
            }
            KeyCode::Char('y') => {
                if let Some(body) = chat.selected_message().and_then(|m| m.body.clone()).filter(|b| !b.is_empty()) {
                    system::copy(&body);
                    self.toast("Copied the message.");
                }
            }
            KeyCode::Char('v') => {
                let images: Vec<Image> = chat
                    .selected_message()
                    .and_then(|m| m.images.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|i| i.url.is_some())
                    .collect();
                if !images.is_empty() {
                    let author = chat.partner.as_ref().map(|p| p.username.clone()).unwrap_or_default();
                    self.viewer = Some(Viewer { images, index: 0, author });
                }
            }
            KeyCode::Char('x') => {
                let on = !chat.is_archived;
                self.stash(handle.to_string(), on);
            }
            KeyCode::Char('a') => self.navigate(Screen::Profile(handle.to_string(), Tab::Commits)),
            KeyCode::Char('R') => self.open_chat(handle),
            _ => return false,
        }
        true
    }

    fn move_chat(&mut self, handle: &str, delta: isize) {
        let Some(chat) = self.remotes.chats.get_mut(handle) else { return };
        if chat.messages.is_empty() {
            return;
        }
        let last = chat.messages.len() as isize - 1;
        let was_first = chat.selected == 0;
        chat.selected = (chat.selected as isize).saturating_add(delta).clamp(0, last) as usize;
        // Scrolling past the oldest message loads the ones before it.
        if (was_first || chat.selected < 5) && delta < 0 {
            self.load_older(handle);
        }
    }

    fn start_typing(&mut self, handle: &str) {
        let Some(chat) = self.remotes.chats.get_mut(handle) else { return };
        let who = chat
            .partner
            .as_ref()
            .map(|p| p.username.clone())
            .unwrap_or_else(|| handle.to_string());
        let notice = access_notice(&chat.access, &who).or_else(|| (!chat.loaded).then(|| "Still loading.".to_string()));
        match notice {
            Some(notice) => self.error(notice),
            None => chat.typing = true,
        }
    }

    fn compose_key(&mut self, handle: &str, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let Some(chat) = self.remotes.chats.get_mut(handle) else { return };
        match key.code {
            KeyCode::Esc => {
                // The first Esc drops a reply, the next stops typing. The draft stays either way.
                if chat.reply_to.is_some() {
                    chat.reply_to = None;
                } else {
                    chat.typing = false;
                }
            }
            KeyCode::Enter if alt || ctrl => chat.insert('\n'),
            KeyCode::Enter => self.send_message(handle),
            KeyCode::Char('j') if ctrl => chat.insert('\n'),
            KeyCode::Char('u') if ctrl => {
                chat.input.drain(..chat.cursor);
                chat.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let mut start = chat.cursor;
                while start > 0 && chat.input[start - 1].is_whitespace() {
                    start -= 1;
                }
                while start > 0 && !chat.input[start - 1].is_whitespace() {
                    start -= 1;
                }
                chat.input.drain(start..chat.cursor);
                chat.cursor = start;
            }
            KeyCode::Char('a') if ctrl => chat.cursor = 0,
            KeyCode::Char('e') if ctrl => chat.cursor = chat.input.len(),
            KeyCode::Char(c) if !ctrl && !alt => chat.insert(c),
            KeyCode::Backspace => {
                if chat.cursor > 0 {
                    chat.cursor -= 1;
                    chat.input.remove(chat.cursor);
                }
            }
            KeyCode::Delete => {
                if chat.cursor < chat.input.len() {
                    chat.input.remove(chat.cursor);
                }
            }
            KeyCode::Left => chat.cursor = chat.cursor.saturating_sub(1),
            KeyCode::Right => chat.cursor = (chat.cursor + 1).min(chat.input.len()),
            KeyCode::Home => chat.cursor = 0,
            KeyCode::End => chat.cursor = chat.input.len(),
            _ => {}
        }
    }

    fn send_message(&mut self, handle: &str) {
        let Some(chat) = self.remotes.chats.get_mut(handle) else { return };
        let body = chat.text().trim().to_string();
        if body.is_empty() || chat.sending {
            return;
        }
        chat.sending = true;
        let (client_id, reply_to) = (uuid(), chat.reply_to);
        let (api, handle) = (self.api.clone(), handle.to_string());
        self.spawn(async move {
            let result = api.push_remote(&handle, &body, &client_id, reply_to).await.map_err(failure);
            Msg::Remote(RemoteMsg::Sent { handle, result })
        });
    }

    fn react(&mut self, index: usize, handle: &str) {
        let me = self.me.as_ref().map(|me| me.user.id.clone()).unwrap_or_default();
        let Some(message) = self.remotes.chats.get(handle).and_then(|c| c.selected_message()) else {
            return;
        };
        let emoji = REACTIONS[index];
        let on = !message.reactions.iter().any(|r| r.user_id == me && r.emoji == emoji);
        let (id, api, handle) = (message.id, self.api.clone(), handle.to_string());
        self.spawn(async move {
            let result = api.react(id, emoji, on).await.map_err(failure);
            Msg::Remote(RemoteMsg::Reacted {
                handle,
                message: id,
                result,
            })
        });
    }

    // Starting a remote -------------------------------------------------------------------------------------------

    fn open_picker(&mut self) {
        self.remotes.picker = Some(Picker {
            query: String::new(),
            candidates: Vec::new(),
            selected: 0,
            loading: false,
            error: None,
            asked: None,
            edited: None,
        });
        self.ask_candidates();
    }

    fn ask_candidates(&mut self) {
        let Some(picker) = self.remotes.picker.as_mut() else { return };
        let term = picker.query.trim().to_string();
        if picker.asked.as_deref() == Some(term.as_str()) {
            return;
        }
        picker.asked = Some(term.clone());
        picker.loading = true;
        let api = self.api.clone();
        self.spawn(async move {
            let result = api.remote_candidates(&term).await.map_err(failure);
            Msg::Remote(RemoteMsg::Candidates { term, result })
        });
    }

    fn picker_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(picker) = self.remotes.picker.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.remotes.picker = None,
            KeyCode::Down | KeyCode::Tab => {
                picker.selected = (picker.selected + 1).min(picker.candidates.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::BackTab => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Char('n') if ctrl => {
                picker.selected = (picker.selected + 1).min(picker.candidates.len().saturating_sub(1));
            }
            KeyCode::Char('p') if ctrl => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Backspace => {
                picker.query.pop();
                picker.edited = Some(Instant::now());
            }
            KeyCode::Enter => {
                let Some(candidate) = picker.candidates.get(picker.selected) else {
                    return;
                };
                match candidate.reason.clone().filter(|r| !r.is_empty()) {
                    Some(reason) => self.error(reason),
                    None => {
                        let handle = candidate.user.username.clone();
                        self.remotes.picker = None;
                        self.navigate(Screen::Remote(handle));
                    }
                }
            }
            KeyCode::Char(c) if !ctrl => {
                picker.query.push(c);
                picker.edited = Some(Instant::now());
            }
            _ => {}
        }
    }
}

impl Chat {
    fn insert(&mut self, c: char) {
        if self.input.len() < MAX_MESSAGE {
            self.input.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// What every answer about a conversation says besides its messages.
    fn apply_state(&mut self, thread: &Thread) {
        self.reactions_version = thread.reactions_version.max(self.reactions_version);
        self.read_through = thread.read_through_id.max(self.read_through);
        self.partner_read_through = thread.partner_read_through_id;
        self.partner_read_at = thread.partner_read_at;
        self.partner_typing = thread.partner_typing;
        self.is_archived = thread.is_archived;
        self.access = thread.access.clone();
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEventState;

    use super::*;
    use crate::api::Api;
    use crate::config::Config;

    #[test]
    fn a_client_id_is_a_version_4_uuid() {
        for _ in 0..50 {
            let id = uuid();
            let parts: Vec<&str> = id.split('-').collect();
            assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
            assert!(parts[2].starts_with('4'));
            assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
            assert!(id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
        }
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
    }

    #[tokio::test]
    async fn writing_edits_the_draft_and_respects_what_the_composer_allows() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Api::new("http://127.0.0.1:9".into(), None), Config::default(), tx);
        let chat = Chat {
            loaded: true,
            access: "closed".into(),
            ..Default::default()
        };
        app.remotes.chats.insert("petox03".into(), chat);
        app.stack = vec![Screen::Remote("petox03".into())];

        // A closed conversation says why, and doesn't start typing.
        press(&mut app, KeyCode::Char('i'));
        assert!(!app.remotes.chats["petox03"].typing);
        assert!(
            app.toast
                .as_ref()
                .is_some_and(|(m, error, _)| *error && m.contains("isn't taking remotes"))
        );

        app.remotes.chats.get_mut("petox03").unwrap().access = "open".into();
        press(&mut app, KeyCode::Char('i'));
        assert!(app.remotes.chats["petox03"].typing);
        for c in "hi g!".chars() {
            // g, q and ? are shortcuts everywhere else; while writing they are letters.
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.remotes.chats["petox03"].text(), "hi ?g");
        assert!(!app.help);

        // Esc stops writing and keeps the draft.
        press(&mut app, KeyCode::Esc);
        assert!(!app.remotes.chats["petox03"].typing);
        assert_eq!(app.remotes.chats["petox03"].text(), "hi ?g");
        assert_eq!(app.screen(), &Screen::Remote("petox03".into()));
    }

    #[tokio::test]
    async fn remotes_need_a_signed_in_terminal() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Api::new("http://127.0.0.1:9".into(), None), Config::default(), tx);
        app.navigate(Screen::Remotes);
        assert_ne!(app.screen(), &Screen::Remotes);
        assert!(app.toast.as_ref().is_some_and(|(m, error, _)| *error && m.contains("Sign in")));
    }
}
