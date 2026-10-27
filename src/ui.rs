use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::activity;
use crate::api::Log;
use crate::app::*;
use crate::big::{self, Size};
use crate::config;
use crate::gfx;
use crate::icons;
use crate::model::*;
use crate::text::{self, body_lines, count, day_label, relative_time, truncate, width, wrap_plain};
use crate::theme::{self, Theme, rgb};

mod remotes_view;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const LIST_BODY_LINES: usize = 10;

/// Whether the terminal has room to spare: wider columns, bigger avatars, more air between commits.
static ROOMY: AtomicBool = AtomicBool::new(false);

fn roomy() -> bool {
    ROOMY.load(Ordering::Relaxed)
}

/// The feed's avatars, in cells: columns, then rows. An odd number of columns, so the rail runs from the middle one,
/// with the round picture centered across them.
fn avatar_size() -> (u16, u16) {
    if roomy() { (7, 3) } else { (5, 2) }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    big::reset();
    draw_frame(f, app);
    big::finish(f.buffer_mut());
}

fn draw_frame(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.buffer_mut().set_style(area, Style::default().bg(Theme::shell()).fg(Theme::ink()));
    if area.width < 30 || area.height < 10 {
        f.render_widget(Paragraph::new("/dev/null needs a bigger terminal.").style(Theme::dim()), area);
        return;
    }

    let main = Rect {
        height: area.height - 1,
        ..area
    };
    let status = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };

    // The feed comes first: as the terminal narrows (or its font grows), the sidebar folds to icons before the right
    // column goes, and the feed keeps at least 72 columns.
    let w = area.width;
    let roomy = w >= 176 && area.height >= 44;
    ROOMY.store(roomy, Ordering::Relaxed);
    let (sidebar_w, right_w, center_max): (u16, u16, u16) = if roomy {
        (30, 46, 116)
    } else if w >= 140 {
        (25, 40, 100)
    } else if w >= 116 {
        (5, 36, 100)
    } else {
        (5, 0, 100)
    };
    let center_w = (w - sidebar_w - right_w - 2).min(center_max);
    let total = sidebar_w + 1 + center_w + 1 + right_w;
    let left = area.x + (w - total) / 2;

    let sidebar = Rect {
        x: left,
        width: sidebar_w,
        ..main
    };
    let center = Rect {
        x: left + sidebar_w + 1,
        width: center_w,
        ..main
    };
    let right = Rect {
        x: center.x + center_w + 1,
        width: right_w,
        ..main
    };

    // Hairlines either side of the center column, as on the web.
    let buf = f.buffer_mut();
    for y in main.y..main.y + main.height {
        for x in [center.x - 1, center.x + center_w] {
            if x < area.x + area.width {
                buf[(x, y)].set_symbol("│").set_fg(Theme::hairline());
            }
        }
    }

    draw_sidebar(f, app, sidebar);
    draw_center(f, app, center);
    if right_w > 0 {
        draw_right(f, app, right);
    }
    draw_status(f, app, status);

    if app.help {
        draw_help(f, app, area);
    }
    if let Some(viewer) = &app.viewer {
        draw_viewer(f, viewer, area);
    }
    if app.achievements
        && let Some(profile) = app.feed().and_then(|feed| feed.profile.clone())
    {
        draw_achievements(f, &profile, area);
    }
    if let Some(confirm) = &app.confirm {
        draw_confirm(f, confirm, area);
    }
    if app.composer.is_some() {
        draw_composer(f, app, center.union(right).intersection(area), area);
    }
    if app.remotes.picker.is_some() {
        remotes_view::draw_picker(f, app, area);
    }
    if app.input.is_some() {
        draw_input(f, app, area);
    }
}

// Sidebar ------------------------------------------------------------------------------------------------------

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let wide = area.width >= 12;
    let buf = f.buffer_mut();

    // The wordmark, large where the terminal can, and the web's tagline under it.
    let ink = Style::default().fg(Theme::ink()).add_modifier(Modifier::BOLD);
    let lane = Style::default().fg(Theme::lane(0)).add_modifier(Modifier::BOLD);
    let x = area.x + if wide { 3 } else { 1 };
    let wordmark = wide
        .then(|| {
            (
                big::rows("/", Size::Double, ink),
                big::rows("/", Size::Double, lane),
                big::rows("/dev/null", Size::Large, ink),
            )
        })
        .and_then(|parts| match parts {
            (Some(a), Some(b), Some(c)) => Some([a, b, c]),
            _ => None,
        });
    // Only whole: cut short, it reads as a typo.
    const TAGLINE: &str = "a social network for devs";
    let fits = wide && x + width(TAGLINE) as u16 <= area.x + area.width;
    let mut nav_y = area.y + 4;
    if let Some([a, b, c]) = wordmark {
        let top = Line::from(vec![a.0, b.0, Span::raw(" "), c.0]);
        let bottom = Line::from(vec![a.1, b.1, Span::raw(" "), c.1]);
        buf.set_line(x, area.y + 1, &top, area.width - 3);
        buf.set_line(x, area.y + 2, &bottom, area.width - 3);
        if fits {
            buf.set_string(x, area.y + 3, TAGLINE, Theme::faint_style());
        }
        nav_y = area.y + 5;
    } else {
        let mut logo = vec![Span::styled("/", ink), Span::styled("/", lane)];
        if wide {
            logo.push(Span::styled("  /dev/null", Theme::bold()));
        }
        buf.set_line(x, area.y + 1, &Line::from(logo), area.width);
        if fits {
            buf.set_string(x, area.y + 2, TAGLINE, Theme::faint_style());
        }
    }

    let screen = app.screen();
    let remotes = match app.remotes.unread {
        0 => "Remotes".to_string(),
        n => format!("Remotes ({n})"),
    };
    let icon = icons::get();
    let items: [(&str, &str, bool, &str); 6] = [
        (icon.log, "Log", matches!(screen, Screen::Log(_)), "g l"),
        (
            icon.notifications,
            "Notifications",
            matches!(screen, Screen::Notifications(_)),
            "g n",
        ),
        (icon.remotes, &remotes, matches!(screen, Screen::Remotes | Screen::Remote(_)), "g r"),
        (
            icon.profile,
            "Profile",
            matches!(screen, Screen::Profile(u, _) if Some(u.clone()) == app.username()),
            "g p",
        ),
        (icon.starred, "Starred", *screen == Screen::Starred, "g b"),
        (icon.settings, "Settings", *screen == Screen::Settings, "g s"),
    ];

    let mut y = nav_y;
    for (icon, label, active, keys) in items {
        if y + 1 >= area.y + area.height {
            break;
        }
        let row = Rect {
            x: area.x + 1,
            y,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        let style = if active {
            Style::default().fg(Theme::on_primary_soft()).bg(Theme::primary_soft())
        } else {
            Theme::dim()
        };
        buf.set_style(row, style);
        let mut spans = vec![Span::styled(format!(" {icon}{}", icons::gap()), style)];
        if wide {
            spans.push(Span::styled(
                format!(" {label}"),
                if active {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    Theme::text()
                },
            ));
            let used = 4 + label.len() as u16;
            if used + keys.len() as u16 + 2 <= row.width {
                let pad = row.width - used - keys.len() as u16 - 1;
                spans.push(Span::raw(" ".repeat(pad as usize)));
                spans.push(Span::styled(keys, if active { style } else { Theme::faint_style() }));
            }
        }
        buf.set_line(row.x, y, &Line::from(spans), row.width);
        y += 2;
    }

    // The commit button.
    if y + 1 < area.y + area.height {
        let row = Rect {
            x: area.x + 1,
            y,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        let style = Style::default()
            .fg(Theme::on_primary())
            .bg(Theme::primary())
            .add_modifier(Modifier::BOLD);
        buf.set_style(row, style);
        let label = if wide {
            format!(" {}{} Commit", icon.commit, icons::gap())
        } else {
            format!(" {}{}", icon.commit, icons::gap())
        };
        buf.set_line(row.x, y, &Line::from(Span::styled(label, style)), row.width);
        if wide {
            buf.set_string(row.x + row.width - 2, y, "c", style);
        }
    }

    // Who this is.
    let bottom = area.y + area.height - 2;
    let line = match (&app.me, app.username()) {
        (Some(me), _) => Line::from(vec![
            Span::raw(" "),
            small_avatar(&me.user),
            Span::styled(
                if wide {
                    truncate(&format!("@{}", me.user.username), area.width as usize - 5)
                } else {
                    String::new()
                },
                Theme::dim(),
            ),
        ]),
        (None, Some(username)) if wide => Line::from(vec![
            Span::styled(" ○\u{a0}", Theme::dim()),
            Span::styled(truncate(&format!("@{username}"), area.width as usize - 5), Theme::faint_style()),
        ]),
        _ => Line::from(Span::styled(if wide { " ○\u{a0}guest" } else { " ○" }, Theme::faint_style())),
    };
    buf.set_line(area.x + 1, bottom, &line, area.width.saturating_sub(1));
}

// Center -------------------------------------------------------------------------------------------------------

fn draw_center(f: &mut Frame, app: &mut App, area: Rect) {
    let screen = app.screen().clone();
    let mut y = area.y;

    match &screen {
        Screen::Log(active) => {
            draw_tabs(
                f.buffer_mut(),
                Rect {
                    y: y + 1,
                    height: 2,
                    ..area
                },
                &Log::ALL.map(|l| (l.title(), l == *active)),
            );
            y += 3;
            hairline(f.buffer_mut(), area.x, y, area.width);
            y += 1;
            y = draw_composer_bar(f.buffer_mut(), app, Rect { y, height: 3, ..area });
            hairline(f.buffer_mut(), area.x, y, area.width);
            y += 1;
        }
        Screen::Profile(username, replies) => {
            y = draw_header(f.buffer_mut(), app, Rect { y, height: 1, ..area }, &format!("@{username}"));
            let profile = app.feed().and_then(|feed| feed.profile.clone());
            if let Some(profile) = profile {
                y = draw_profile_card(
                    f.buffer_mut(),
                    app,
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: area.height.saturating_sub(y - area.y),
                    },
                    &profile,
                );
            }
            draw_tabs(
                f.buffer_mut(),
                Rect { y, height: 2, ..area },
                &Tab::ALL.map(|t| (t.title(), t == *replies)),
            );
            y += 2;
            hairline(f.buffer_mut(), area.x, y, area.width);
            y += 1;
        }
        Screen::Settings => {
            y = draw_header(f.buffer_mut(), app, Rect { y, height: 1, ..area }, "Settings");
            draw_settings(
                f,
                app,
                Rect {
                    y,
                    height: area.height.saturating_sub(y - area.y),
                    ..area
                },
            );
            return;
        }
        Screen::Remotes | Screen::Remote(_) => {
            remotes_view::draw(f, app, area);
            return;
        }
        Screen::Notifications(active) => {
            y = draw_header(f.buffer_mut(), app, Rect { y, height: 1, ..area }, "Notifications");
            draw_tabs(
                f.buffer_mut(),
                Rect { y, height: 2, ..area },
                &Filter::ALL.map(|filter| (filter.title(), filter == *active)),
            );
            y += 2;
            hairline(f.buffer_mut(), area.x, y, area.width);
            y += 1;
        }
        other => {
            y = draw_header(f.buffer_mut(), app, Rect { y, height: 1, ..area }, &other.title());
        }
    }

    let new_count = app.feed().map(|f| f.new_count).unwrap_or(0);
    if new_count > 0 {
        let row = Rect { y, height: 1, ..area };
        let style = Style::default().fg(Theme::on_primary_soft()).bg(Theme::primary_soft());
        f.buffer_mut().set_style(row, style);
        let label = format!("↑\u{a0}{} new commit{} · R", new_count, if new_count == 1 { "" } else { "s" });
        let x = area.x + (area.width.saturating_sub(width(&label) as u16)) / 2;
        f.buffer_mut().set_string(x, y, label, style.add_modifier(Modifier::BOLD));
        y += 1;
    }

    let feed_area = Rect {
        y,
        height: area.height.saturating_sub(y - area.y),
        ..area
    };
    app.feed_area = feed_area;
    draw_feed(f, app, feed_area);
}

fn hairline(buf: &mut Buffer, x: u16, y: u16, w: u16) {
    buf.set_string(x, y, "─".repeat(w as usize), Style::default().fg(Theme::hairline()));
}

fn draw_tabs(buf: &mut Buffer, area: Rect, tabs: &[(&str, bool)]) {
    let n = tabs.len() as u16;
    let cell = area.width / n;
    for (i, (label, active)) in tabs.iter().enumerate() {
        let x = area.x + cell * i as u16;
        let lw = width(label) as u16;
        let lx = x + cell.saturating_sub(lw) / 2;
        let style = if *active { Theme::bold() } else { Theme::dim() };
        buf.set_string(lx, area.y, *label, style);
        if *active {
            buf.set_string(
                lx.saturating_sub(1),
                area.y + 1,
                "━".repeat(lw as usize + 2),
                Style::default().fg(Theme::primary()),
            );
        }
    }
}

fn draw_header(buf: &mut Buffer, app: &App, area: Rect, title: &str) -> u16 {
    let back = app.stack.len() > 1;
    let arrow = Span::styled(if back { " ←\u{a0}" } else { "   " }, Theme::dim());
    // Twice the size where the terminal can, the arrow and esc on the title's baseline.
    let room = (area.width as usize).saturating_sub(8);
    let rows = big::rows(title, Size::Double, Theme::bold()).filter(|(_, _, w)| *w <= room);
    let tall = rows.is_some();
    match rows {
        Some((top, bottom, _)) => {
            buf.set_line(area.x, area.y + 1, &Line::from(vec![Span::raw("   "), top]), area.width);
            buf.set_line(area.x, area.y + 2, &Line::from(vec![arrow, bottom]), area.width);
        }
        None => {
            let line = Line::from(vec![arrow, Span::styled(title.to_string(), Theme::bold())]);
            buf.set_line(area.x, area.y + 1, &line, area.width);
        }
    }
    let baseline = area.y + 1 + u16::from(tall);
    if back {
        let hint = "esc";
        buf.set_string(area.x + area.width - hint.len() as u16 - 1, baseline, hint, Theme::faint_style());
    }
    hairline(buf, area.x, baseline + 1, area.width);
    baseline + 2
}

/// The "What did you ship?" box at the top of the log.
fn draw_composer_bar(buf: &mut Buffer, app: &App, area: Rect) -> u16 {
    let lane = app.me.as_ref().map(|m| Theme::user_lane(&m.user)).unwrap_or(Theme::rail());
    match app.me.as_ref().and_then(|me| avatar_row(&me.user, 2, 1, 0)) {
        Some(avatar) => {
            buf.set_line(area.x + 2, area.y + 1, &Line::from(avatar), 2);
        }
        None => {
            buf.set_string(area.x + 2, area.y + 1, "⦿", Style::default().fg(lane));
        }
    }
    let draft = app.draft.as_ref().map(|d| d.iter().collect::<String>());
    let (text, style) = match &draft {
        Some(d) => (truncate(d.lines().next().unwrap_or(""), area.width as usize - 20), Theme::text()),
        None => ("What did you ship?".to_string(), Theme::dim()),
    };
    buf.set_string(area.x + 5, area.y + 1, text, style);

    let button = " Commit  c ";
    let bx = area.x + area.width - width(button) as u16 - 1;
    buf.set_string(bx, area.y + 1, button, Style::default().fg(Theme::ink()).bg(Theme::primary_muted()));
    area.y + 3
}

fn draw_profile_card(buf: &mut Buffer, app: &App, area: Rect, profile: &Profile) -> u16 {
    let user = &profile.data;
    let inner = area.width.saturating_sub(4) as usize;
    let mut lines: Vec<Line> = Vec::new();

    // The name twice the size where the terminal can, its handle and badges on the second row.
    let dot = Span::styled("●\u{a0}", Style::default().fg(Theme::user_lane(user)));
    let mut name = match big::rows(user.display_name(), Size::Double, Theme::bold()).filter(|(_, _, w)| *w + 2 <= inner) {
        Some((top, bottom, _)) => {
            lines.push(Line::from(vec![dot, top]));
            vec![Span::raw("  "), bottom]
        }
        None => vec![dot, Span::styled(user.display_name().to_string(), Theme::bold())],
    };
    name.push(Span::styled(format!("  @{}", user.username), Theme::dim()));
    if profile.is_muting == Some(true) {
        name.push(Span::styled("  muted", Style::default().fg(Theme::lane(5)).bg(Theme::wash())));
    }
    if profile.follows_you == Some(true) {
        name.push(Span::styled("  follows you", Style::default().fg(Theme::muted()).bg(Theme::wash())));
    }
    lines.push(Line::from(name));

    if let Some(bio) = user.bio.as_deref().filter(|b| !b.trim().is_empty()) {
        lines.extend(wrap_plain(bio.trim(), inner, Theme::text()).into_iter().take(3));
    }

    let mut meta: Vec<String> = Vec::new();
    for value in [&user.company, &user.location, &user.website].into_iter().flatten() {
        if !value.trim().is_empty() {
            meta.push(value.trim().trim_start_matches("https://").trim_end_matches('/').to_string());
        }
    }
    if !meta.is_empty() {
        lines.push(Line::from(Span::styled(truncate(&meta.join("  ·  "), inner), Theme::dim())));
    }

    let mut counts = Vec::new();
    for (n, label) in [
        (user.commits_count, "commits"),
        (user.following_count, "following"),
        (user.followers_count, "followers"),
    ] {
        if let Some(n) = n {
            counts.push(Span::styled(count(n), Theme::bold()));
            counts.push(Span::styled(format!(" {label}   "), Theme::dim()));
        }
    }
    let mine = app.username().as_deref() == Some(user.username.as_str());
    if !mine && app.signed() {
        let following = profile.is_following == Some(true);
        counts.push(Span::styled(
            if following { " Following · f " } else { " Follow · f " },
            if following {
                Style::default().fg(Theme::ink()).bg(Theme::wash())
            } else {
                Style::default().fg(Theme::on_primary()).bg(Theme::primary())
            },
        ));
    }
    if !mine && app.me.as_ref().is_some_and(|me| me.remotes.is_some()) {
        counts.push(Span::raw(" "));
        counts.push(Span::styled(" Remote · M ", Style::default().fg(Theme::ink()).bg(Theme::wash())));
    }
    lines.push(Line::from(counts));

    // Activity, streaks and standing, when there's room for them above the commits.
    if area.height >= 30 {
        let lane = Theme::user_lane_hex(user);
        let room = inner.saturating_sub(if gfx::enabled() { 5 } else { 0 });
        let mut extra = Vec::new();
        if let Some(activity) = &profile.activity {
            extra.extend(activity::graph(activity, lane, room));
        }
        if let Some(line) = activity::standing(profile.streaks.as_ref(), profile.achievements.as_ref(), lane) {
            extra.push(line);
        }
        if let Some(line) = profile.achievements.as_ref().and_then(|a| activity::badges(a, room)) {
            extra.push(line);
        }
        if !extra.is_empty() {
            lines.push(Line::default());
            lines.extend(extra);
        }
    }

    if let (Some(top), Some(bottom)) = (avatar_row(user, 4, 2, 0), avatar_row(user, 4, 2, 1))
        && lines.len() >= 2
    {
        // The avatar stands in for the dot, and the space under it.
        lines[0].spans.remove(0);
        if lines[1].spans.first().is_some_and(|s| s.content == "  ") {
            lines[1].spans.remove(0);
        }
        for (i, line) in lines.iter_mut().enumerate() {
            let lead = match i {
                0 => top.clone(),
                1 => bottom.clone(),
                _ => Span::raw("    "),
            };
            line.spans.insert(0, Span::raw(" "));
            line.spans.insert(0, lead);
        }
    }

    let mut y = area.y + 1;
    for line in lines {
        if y >= area.y + area.height.saturating_sub(3) {
            break;
        }
        buf.set_line(area.x + 2, y, &line, area.width - 4);
        y += 1;
    }
    y + 1
}

// The feed -----------------------------------------------------------------------------------------------------

struct Rendered {
    lines: Vec<Line<'static>>,
    /// Lines from here on are the item itself, and get highlighted when it's selected.
    highlight_from: usize,
    /// Lines at the end that follow the item rather than belong to it, like the thread's Replies divider.
    trailer: usize,
    /// The commit's header, which a theme with title bars draws as one.
    title: Option<usize>,
}

fn draw_feed(f: &mut Frame, app: &mut App, area: Rect) {
    let spinner = SPINNER[app.tick % SPINNER.len()];
    let screen = app.screen().clone();
    let pasteis = app.me.as_ref().is_some_and(|m| m.pasteis_mode);
    let app_username = app.username();
    let Some(feed) = app.feeds.get(&screen) else { return };

    if feed.items.is_empty() {
        let (message, style) = if feed.loading || !feed.loaded {
            (format!("{spinner} Reading from /dev/null…"), Theme::dim())
        } else if let Some(error) = &feed.error {
            (format!("{error}\n\nR to try again"), Style::default().fg(Theme::destructive()))
        } else if let Some(notice) = &feed.notice {
            (notice.clone(), Theme::dim())
        } else {
            (empty_message(&screen).to_string(), Theme::dim())
        };
        let y = area.y + area.height / 3;
        let p = Paragraph::new(message)
            .style(style)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        f.render_widget(
            p,
            Rect {
                x: area.x + 4,
                y,
                width: area.width.saturating_sub(8),
                height: area.height.saturating_sub(y - area.y),
            },
        );
        return;
    }

    let chronological = matches!(
        screen,
        Screen::Log(Log::Latest) | Screen::Log(Log::Following) | Screen::Profile(_, Tab::Commits | Tab::Replies)
    );
    let mut all: Vec<Line<'static>> = Vec::new();
    let mut offsets = Vec::with_capacity(feed.items.len());
    let mut last_day: Option<String> = None;
    let mut titles: Vec<(usize, bool)> = Vec::new();

    for (i, item) in feed.items.iter().enumerate() {
        let selected = i == feed.selected;
        let mut rendered = match item {
            Item::Commit(commit, role) => {
                let next_is_thread = matches!(feed.items.get(i + 1), Some(Item::Commit(_, Role::Focus | Role::Ancestor)));
                render_commit(
                    feed,
                    commit,
                    *role,
                    area.width as usize,
                    selected,
                    pasteis,
                    next_is_thread && *role == Role::Ancestor,
                )
            }
            Item::Notification(n) => render_notification(n, area.width as usize, selected),
            Item::User(user) => render_person(user, area.width as usize, selected, app_username.as_deref()),
        };

        if chronological {
            let at = item.commit().and_then(|c| c.created_at);
            let pinned = item.commit().and_then(|c| c.is_pinned).unwrap_or(false);
            if let (Some(at), false) = (at, pinned) {
                let label = day_label(at);
                if last_day.as_deref() != Some(label.as_str()) {
                    let mut lines = vec![
                        Line::from(Span::styled(format!("    {label}"), Theme::faint_style())),
                        hairline_line(area.width),
                    ];
                    rendered.highlight_from += lines.len();
                    rendered.title = rendered.title.map(|t| t + lines.len());
                    lines.append(&mut rendered.lines);
                    rendered.lines = lines;
                    last_day = Some(label);
                }
            }
        }

        let start = all.len();
        if let Some(t) = rendered.title.filter(|_| window(selected)) {
            titles.push((start + t, selected));
        }
        offsets.push((start + rendered.highlight_from, start + rendered.lines.len() - rendered.trailer));
        all.extend(rendered.lines);
    }

    if feed.loading {
        all.push(Line::from(Span::styled(format!("    {spinner} loading more…"), Theme::dim())));
    } else if feed.cursor.is_none() && !matches!(screen, Screen::Thread(_)) {
        let eof = "EOF";
        let pad = (area.width as usize).saturating_sub(eof.len()) / 2;
        all.push(Line::default());
        all.push(Line::from(Span::styled(format!("{}{eof}", " ".repeat(pad)), Theme::faint_style())));
        all.push(Line::default());
    } else if let Some(notice) = &feed.notice {
        all.push(Line::from(Span::styled(format!("    {notice}"), Theme::dim())));
    }
    if let (Screen::Thread(_), Some(notice)) = (&screen, &feed.notice)
        && feed.cursor.is_none()
        && !feed.loading
    {
        all.push(Line::from(Span::styled(format!("    {notice}"), Theme::dim())));
    }

    // Keep the selected commit in view.
    let height = area.height as usize;
    let (sel_start, sel_end) = offsets.get(feed.selected).copied().unwrap_or((0, 0));
    let feed = app.feeds.get_mut(&screen).expect("feed");
    let mut scroll = feed.scroll;
    if feed.selected == 0 {
        scroll = 0;
    }
    if sel_end > scroll + height {
        scroll = sel_end.saturating_sub(height);
    }
    if sel_start < scroll || sel_end - sel_start > height {
        scroll = sel_start.saturating_sub(if feed.selected == 0 { sel_start } else { 0 });
    }
    scroll = scroll.min(all.len().saturating_sub(1));
    feed.scroll = scroll;
    feed.offsets = offsets;

    let buf = f.buffer_mut();
    for (row, line) in all.iter().skip(scroll).take(height).enumerate() {
        let y = area.y + row as u16;
        let index = scroll + row;
        if index >= sel_start && index < sel_end {
            buf.set_style(
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
                Style::default().bg(Theme::highlight()),
            );
        }
        buf.set_line(area.x, y, line, area.width);
        if let Some(&(_, active)) = titles.iter().find(|(at, _)| *at == index) {
            draw_title_bar(buf, area, y, active);
        }
    }

    // A scrollbar, when there's more than a screen.
    if all.len() > height && height > 2 {
        let thumb = ((height * height) / all.len()).max(1);
        let top = (scroll * (height - thumb)) / all.len().saturating_sub(height).max(1);
        for i in 0..thumb {
            let y = area.y + (top + i).min(height - 1) as u16;
            buf[(area.x + area.width - 1, y)].set_symbol("▐").set_fg(Theme::rail());
        }
    }
}

/// What a title bar has on the right, as Windows 98 has them: minimize, maximize and close.
const BUTTONS: &str = " _□\u{a0}× ";

/// Whether a commit's header is drawn as a title bar.
fn window(selected: bool) -> bool {
    Theme::title_bar().is_some_and(|t| selected || t.inactive.is_some())
}

/// Repaints a commit's header as a window title bar: the theme's fade behind it, its text in the title bar's ink,
/// and the buttons on the right. The gutter is left alone, since avatars keep their image id in the colors there.
fn draw_title_bar(buf: &mut Buffer, area: Rect, y: u16, active: bool) {
    let Some(bar) = Theme::title_bar() else { return };
    let ((from, to), ink) = match (active, bar.inactive) {
        (false, Some(inactive)) => inactive,
        _ => (bar.active, bar.active_ink),
    };
    let start = area.x + gutter_width() as u16;
    let end = area.x + area.width.saturating_sub(2);
    let span = end.saturating_sub(start).max(1);
    for x in start..end {
        let t = (x - start) as f32 / span as f32;
        buf[(x, y)].set_bg(rgb(theme::fade(from, to, t))).set_fg(rgb(ink));
    }
    let buttons = end.saturating_sub(BUTTONS.len() as u16);
    for (i, c) in BUTTONS.chars().enumerate() {
        if c != ' ' && buttons + (i as u16) >= start {
            buf[(buttons + i as u16, y)]
                .set_symbol(&c.to_string())
                .set_fg(Theme::ink())
                .set_bg(Theme::popover())
                .modifier = Modifier::BOLD;
        }
    }
}

fn empty_message(screen: &Screen) -> &'static str {
    match screen {
        Screen::Log(Log::Following) => "Nothing here yet. Follow a few developers and their commits show up here.",
        Screen::Log(_) => "Nothing committed yet. Be the first: c",
        Screen::Notifications(Filter::All) => "No notifications. It's quiet in here.",
        Screen::Notifications(_) => "Nothing of that kind. ⇥\u{a0}for the next filter.",
        Screen::Starred => "Nothing starred yet. Press s on a commit to keep it here.",
        Screen::Profile(_, Tab::Replies) => "No replies yet.",
        Screen::Profile(_, Tab::Commits) => "No commits yet. Everything else went to /dev/null.",
        Screen::Profile(_, Tab::Followers) => "No followers yet.",
        Screen::Profile(_, Tab::Following) => "Not following anyone yet.",
        _ => "",
    }
}

fn hairline_line(w: u16) -> Line<'static> {
    Line::from(Span::styled("─".repeat(w as usize), Style::default().fg(Theme::hairline())))
}

fn bar(selected: bool) -> Span<'static> {
    Span::styled(if selected { "▌" } else { " " }, Style::default().fg(Theme::primary()))
}

/// The gutter: a selection bar, the commit's node or the rail running through it, and a gap. Always five spans,
/// wider when there are avatars to make room for, with the node under the avatar's middle. The gap opens with a
/// no-break space, as terminals let a symbol followed by a plain one spill into its cell, off the rail.
fn gutter(selected: bool, mark: Span<'static>) -> Vec<Span<'static>> {
    let (before, after) = match (gfx::enabled(), roomy()) {
        (false, _) => ("", "\u{a0} "),
        (true, false) => ("  ", "\u{a0}  "),
        (true, true) => ("   ", "\u{a0}   "),
    };
    vec![bar(selected), Span::raw(" "), Span::raw(before), mark, Span::raw(after)]
}

fn gutter_width() -> usize {
    match (gfx::enabled(), roomy()) {
        (false, _) => 5,
        (true, false) => 8,
        (true, true) => 10,
    }
}

/// Puts one row of an avatar where a line's gutter has its node or rail.
fn with_avatar(line: Line<'static>, selected: bool, avatar: Span<'static>) -> Line<'static> {
    let rest: Vec<Span<'static>> = line.spans.into_iter().skip(5).collect();
    Line::from([vec![bar(selected), Span::raw(" "), avatar, Span::raw(" ")], rest].concat())
}

fn avatar_row(user: &User, cols: u16, rows: u16, row: u16) -> Option<Span<'static>> {
    let url = user.avatar_url.as_deref().filter(|u| !u.is_empty())?;
    gfx::row(
        &gfx::Key {
            url: gfx::avatar_url(url),
            cols,
            rows,
            ring: Some(Theme::user_lane_hex(user)),
        },
        row,
    )
}

/// A two-cell avatar and a gap, or the dot it stands in for.
fn small_avatar(user: &User) -> Span<'static> {
    match avatar_row(user, 2, 1, 0) {
        Some(avatar) => Span::styled(format!("{} ", avatar.content), avatar.style),
        None => Span::styled("●\u{a0}", Style::default().fg(Theme::user_lane(user))),
    }
}

/// A commit's images, side by side, once the terminal can show pictures.
fn picture_lines(images: &[Image], content: usize, max_rows: u16) -> Option<Vec<Line<'static>>> {
    if !gfx::enabled() || images.iter().all(|i| i.url.is_none()) {
        return None;
    }
    let shown: Vec<&Image> = images.iter().filter(|i| i.url.is_some()).take(3).collect();
    let n = shown.len();
    let each = (content.saturating_sub(n - 1) / n).min(if roomy() { 72 } else { 48 }) as u16;
    let keys: Vec<gfx::Key> = shown
        .iter()
        .map(|image| {
            let (cols, rows) = gfx::fit(image.width.unwrap_or(4), image.height.unwrap_or(3), each, max_rows);
            gfx::Key {
                url: image.url.clone().unwrap_or_default(),
                cols,
                rows,
                ring: None,
            }
        })
        .collect();
    let height = keys.iter().map(|k| k.rows).max().unwrap_or(0);
    let mut lines = Vec::new();
    for row in 0..height {
        let mut spans = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            let span = if row < key.rows {
                gfx::row(key, row).unwrap_or_else(|| Span::styled(" ".repeat(key.cols as usize), Style::default().bg(Theme::wash())))
            } else {
                Span::raw(" ".repeat(key.cols as usize))
            };
            spans.push(span);
        }
        lines.push(Line::from(spans));
    }
    Some(lines)
}

fn render_commit(feed: &Feed, commit: &Commit, role: Role, w: usize, selected: bool, pasteis: bool, rail_after: bool) -> Rendered {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let content = w.saturating_sub(gutter_width() + 2).max(10);

    if commit.is_tombstone() {
        let rail = Span::styled("│", Style::default().fg(Theme::rail()));
        lines.push(Line::from(
            [
                gutter(selected, Span::styled("○", Theme::faint_style())),
                vec![Span::styled(
                    "This commit was deleted.",
                    Theme::faint_style().add_modifier(Modifier::ITALIC),
                )],
            ]
            .concat(),
        ));
        lines.push(Line::from(gutter(selected, if rail_after { rail } else { Span::raw(" ") })));
        if !rail_after {
            lines.push(hairline_line(w as u16));
        }
        return Rendered {
            lines,
            highlight_from: 0,
            trailer: usize::from(!rail_after),
            title: None,
        };
    }

    let author = commit.author.clone().unwrap_or_default();
    let lane = Theme::user_lane(&author);
    let replies: Vec<Commit> = if feed.collapsed.contains(&commit.id) || role != Role::Normal {
        vec![]
    } else if feed.expanded.contains(&commit.id) {
        feed.replies
            .get(&commit.id)
            .cloned()
            .unwrap_or_else(|| commit.reply_preview.clone().map(|p| p.data).unwrap_or_default())
    } else {
        commit.reply_preview.clone().map(|p| p.data).unwrap_or_default()
    };
    // The rail runs from the avatar down to the commit's node on its actions, as on the web.
    let rail = || Span::styled("│", Style::default().fg(lane));

    if roomy() {
        lines.push(Line::from(gutter(selected, Span::raw(" "))));
    }
    if commit.is_pinned == Some(true) && role == Role::Normal {
        lines.push(Line::from(
            [
                gutter(selected, Span::raw(" ")),
                vec![Span::styled("◇\u{a0}Pinned", Theme::faint_style())],
            ]
            .concat(),
        ));
    }

    // Name @username · 2m ............ hash
    let pending = commit.status.as_deref() == Some("pending");
    let node = if pending {
        Span::styled("◌", Style::default().fg(lane))
    } else {
        Span::styled(icons::get().node, Style::default().fg(lane).add_modifier(Modifier::BOLD))
    };
    let hash = commit.hash();
    let time = relative_time(commit.created_at);
    let mut status: Vec<Span<'static>> = Vec::new();
    match commit.status.as_deref() {
        Some("pending") => status.push(Span::styled(" · pending review", Style::default().fg(Theme::lane(5)))),
        Some("rejected") => status.push(Span::styled(" · rejected", Style::default().fg(Theme::destructive()))),
        _ => {}
    }
    // A title bar keeps room on the right for its buttons.
    let content = if window(selected) {
        content.saturating_sub(BUTTONS.len())
    } else {
        content
    };
    let fixed = width(&author.username) + 2 + 3 + width(&time) + hash.len() + 2 + status.iter().map(|s| s.width()).sum::<usize>();
    let room = content.saturating_sub(fixed).max(6);
    // A large name takes two rows, and the rest of the header sits on the second, along its baseline.
    let large = big::rows(&truncate(author.display_name(), room * 2 / 3), Size::Large, Theme::bold());
    let mut header = Vec::new();
    match &large {
        Some((top, bottom, _)) => {
            lines.push(Line::from([gutter(selected, node.clone()), vec![top.clone()]].concat()));
            header.push(bottom.clone());
        }
        None => header.push(Span::styled(truncate(author.display_name(), room), Theme::bold())),
    }
    header.push(Span::styled(format!(" @{}", author.username), Theme::dim()));
    header.push(Span::styled(format!(" · {time}"), Theme::dim()));
    header.extend(status);
    let used: usize = header.iter().map(|s| s.width()).sum();
    if used + hash.len() + 1 < content {
        header.push(Span::raw(" ".repeat(content - used - hash.len())));
        header.push(Span::styled(hash, Theme::faint_style()));
    }
    let mark = if large.is_some() { rail() } else { node };
    lines.push(Line::from([gutter(selected, mark), header].concat()));
    let header_at = lines.len() - 1 - usize::from(large.is_some());
    let content = w.saturating_sub(gutter_width() + 2).max(10);

    let prefix = |lines: &mut Vec<Line<'static>>, body: Vec<Line<'static>>| {
        for line in body {
            let mut spans = gutter(selected, rail());
            spans.extend(line.spans);
            lines.push(Line::from(spans));
        }
    };

    // Body, cut short in lists as the web's preview does.
    let body = commit.body.clone().unwrap_or_default();
    if !body.trim().is_empty() {
        let mut text = text::body_lines_highlighted(&body, content, Theme::text(), &commit.highlighted_blocks());
        if role != Role::Focus && text.len() > LIST_BODY_LINES {
            text.truncate(LIST_BODY_LINES - 2);
            while text.last().is_some_and(|l| l.width() == 0) {
                text.pop();
            }
            text.push(Line::from(Span::styled("Show more ⏎", Theme::link())));
        }
        prefix(&mut lines, text);
    }

    // Images, link card, quote, poll.
    let mut extras: Vec<Line<'static>> = Vec::new();
    let pictures = commit.images.as_deref().and_then(|images| {
        let rows = match (role == Role::Focus, roomy()) {
            (true, true) => 26,
            (true, false) => 20,
            (false, true) => 16,
            (false, false) => 12,
        };
        picture_lines(images, content, rows)
    });
    if let Some(pictures) = pictures {
        extras.extend(pictures);
    } else if let Some(images) = commit.images.as_ref().filter(|i| !i.is_empty()) {
        let sizes: Vec<String> = images
            .iter()
            .map(|i| match (i.width, i.height) {
                (Some(w), Some(h)) => format!("{w}×{h}"),
                _ => "image".into(),
            })
            .collect();
        extras.push(Line::from(vec![
            Span::styled("⊞\u{a0}", Style::default().fg(lane)),
            Span::styled(
                format!(
                    "{} image{} · {}",
                    images.len(),
                    if images.len() == 1 { "" } else { "s" },
                    sizes.join(", ")
                ),
                Theme::dim(),
            ),
            Span::styled("  w opens", Theme::faint_style()),
        ]));
    }
    if let Some(poll) = &commit.poll {
        extras.extend(render_poll(poll, content));
    }
    if let Some(quoted) = &commit.quoted_commit {
        extras.extend(boxed(quote_lines(quoted, content.saturating_sub(4)), content));
    } else if commit.quoted_commit_id.is_some() {
        extras.extend(boxed(
            vec![Line::from(Span::styled("This commit is unavailable.", Theme::faint_style()))],
            content,
        ));
    }
    if let Some(preview) = commit
        .link_preview
        .as_ref()
        .filter(|p| p.title.is_some() || p.description.is_some())
    {
        extras.extend(boxed(link_lines(preview, content.saturating_sub(4)), content));
    }
    if !extras.is_empty() {
        prefix(&mut lines, extras);
    }

    // Actions.
    let starred = commit.starred.unwrap_or(false);
    let mut actions = vec![
        Span::styled(
            if starred { icons::get().star_full } else { icons::get().star },
            Style::default().fg(if starred { Theme::star() } else { Theme::muted() }),
        ),
        Span::styled(
            format!("{}{:<5}", icons::gap(), commit.stars_count.map(count).unwrap_or_default()),
            Theme::dim(),
        ),
    ];
    if pasteis {
        actions.push(Span::styled("⊚", Style::default().fg(Theme::lane(5))));
        actions.push(Span::styled(
            format!("\u{a0}{:<5}", commit.pasteis_count.map(count).unwrap_or_else(|| "0".into())),
            Theme::dim(),
        ));
    }
    let (icon, gap) = (icons::get(), icons::gap());
    actions.push(Span::styled(format!("{}{gap}Quote   ", icon.quote), Theme::dim()));
    let replies_count = commit.replies_count.unwrap_or(0);
    actions.push(Span::styled(
        if replies_count > 0 {
            format!(
                "{}{gap}{} repl{}",
                icon.reply,
                count(replies_count),
                if replies_count == 1 { "y" } else { "ies" }
            )
        } else {
            format!("{}{gap}Reply", icon.reply)
        },
        Theme::dim(),
    ));
    if role == Role::Focus {
        let full = commit
            .created_at
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M · %a %-d %b %Y").to_string())
            .unwrap_or_default();
        prefix(
            &mut lines,
            vec![Line::default(), Line::from(Span::styled(full, Theme::faint_style()))],
        );
    }
    // The node goes on the actions, a stretch of rail below the avatar however short the commit.
    let (cols, rows) = avatar_size();
    if gfx::enabled() {
        while lines.len() <= header_at + rows as usize {
            prefix(&mut lines, vec![Line::default()]);
        }
    }
    let dot = Span::styled(
        if pending { "◌" } else { icons::get().node },
        Style::default().fg(lane).add_modifier(Modifier::BOLD),
    );
    lines.push(Line::from([gutter(selected, dot), actions].concat()));

    // The author's avatar takes the node's place, over the header and the lines below it.
    let avatar: Option<Vec<Span<'static>>> = (0..rows).map(|row| avatar_row(&author, cols, rows, row)).collect();
    for (row, span) in avatar.into_iter().flatten().enumerate() {
        let at = header_at + row;
        lines[at] = with_avatar(std::mem::take(&mut lines[at]), selected, span);
    }

    // Replies under it, as the web shows them: a dotted branch off the node into each one's avatar, then its text and
    // actions beside it. One that quotes the commit says so.
    if !replies.is_empty() {
        let total = replies.len();
        let dotted = Style::default().fg(lane);
        for (i, reply) in replies.iter().enumerate() {
            let last = i + 1 == total;
            let reply_author = reply.author.clone().unwrap_or_default();
            let avatar: Option<Vec<Span<'static>>> = (0..2).map(|row| avatar_row(&reply_author, 4, 2, row)).collect();
            let lead_w = if avatar.is_some() { 5 } else { 2 };
            let inner = content.saturating_sub(lead_w);

            let mut head = vec![
                Span::styled(truncate(reply_author.display_name(), inner / 2), Theme::bold()),
                Span::styled(format!(" @{}", reply_author.username), Theme::dim()),
            ];
            // Quotes come in the preview too: anything that isn't a reply to this commit quoted it.
            if reply.parent_id.as_deref() != Some(commit.id.as_str()) {
                head.push(Span::styled(" quoted", Theme::link()));
            }
            head.push(Span::styled(format!(" · {}", relative_time(reply.created_at)), Theme::dim()));
            let mut rows = vec![Line::from(head)];
            let mut body = body_lines(reply.body.as_deref().unwrap_or(""), inner, Theme::text());
            if body.len() > 4 {
                body.truncate(3);
                body.push(Line::from(Span::styled("…", Theme::dim())));
            }
            rows.extend(body);
            if avatar.is_some() && rows.len() < 2 {
                rows.push(Line::default());
            }
            let starred = reply.starred.unwrap_or(false);
            let mut actions = vec![
                Span::styled(
                    if starred { icons::get().star_full } else { icons::get().star },
                    Style::default().fg(if starred { Theme::star() } else { Theme::muted() }),
                ),
                Span::styled(
                    format!("{}{:<5}", icons::gap(), reply.stars_count.map(count).unwrap_or_default()),
                    Theme::dim(),
                ),
            ];
            if pasteis {
                actions.push(Span::styled("⊚", Style::default().fg(Theme::lane(5))));
                actions.push(Span::styled(
                    format!("\u{a0}{:<5}", reply.pasteis_count.map(count).unwrap_or_else(|| "0".into())),
                    Theme::dim(),
                ));
            }
            actions.push(Span::styled(
                format!("{}{}Reply   ", icons::get().reply, icons::gap()),
                Theme::dim(),
            ));
            if let Some(n) = reply.replies_count.filter(|n| *n > 0) {
                actions.push(Span::styled(
                    format!("›\u{a0}{} repl{}", count(n), if n == 1 { "y" } else { "ies" }),
                    Theme::link(),
                ));
            }
            rows.push(Line::from(actions));

            for (j, row) in rows.into_iter().enumerate() {
                let mut spans = gutter(selected, Span::raw(" "));
                let gap = spans[4].width();
                let (mark, rest) = match (j, last) {
                    (0, false) => ("├", format!("{} ", "┄".repeat(gap - 1))),
                    (0, true) => ("╰", format!("{} ", "┄".repeat(gap - 1))),
                    (_, false) => ("┊", " ".repeat(gap)),
                    (_, true) => (" ", " ".repeat(gap)),
                };
                spans[3] = Span::styled(mark, dotted);
                spans[4] = Span::styled(rest, dotted);
                match (&avatar, j) {
                    (Some(avatar), 0 | 1) => spans.extend([avatar[j].clone(), Span::raw(" ")]),
                    (Some(_), _) => spans.push(Span::raw("     ")),
                    (None, 0) => spans.push(small_avatar(&reply_author)),
                    (None, _) => spans.push(Span::raw("  ")),
                }
                spans.extend(row.spans);
                lines.push(Line::from(spans));
            }
            if !last {
                let mut spans = gutter(selected, Span::styled("┊", dotted));
                spans[4] = Span::raw(" ".repeat(spans[4].width()));
                lines.push(Line::from(spans));
            }
        }
        let shown = total as u64;
        if replies_count > shown && !feed.expanded.contains(&commit.id) {
            lines.push(Line::from(
                [
                    gutter(selected, Span::raw(" ")),
                    vec![Span::styled(format!("   {} more · o", replies_count - shown), Theme::link())],
                ]
                .concat(),
            ));
        }
    } else if feed.expanded.contains(&commit.id) && replies_count > 0 && role == Role::Normal {
        lines.push(Line::from(
            [gutter(selected, rail()), vec![Span::styled("loading replies…", Theme::dim())]].concat(),
        ));
    }

    if rail_after {
        lines.push(Line::from(gutter(selected, rail())));
    } else {
        lines.push(Line::from(gutter(selected, Span::raw(" "))));
        lines.push(hairline_line(w as u16));
        if role == Role::Focus {
            // The web's reply box under an open commit, as a prompt.
            let who = author.username.clone();
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("↩\u{a0}", Theme::link()),
                Span::styled(format!("Reply to @{who}…"), Theme::dim()),
                Span::raw("   "),
                key_chip("⏎"),
                Span::styled(" or ", Theme::faint_style()),
                key_chip("r"),
            ]));
            lines.push(hairline_line(w as u16));
            lines.push(Line::from(Span::styled(
                format!(
                    "    Replies{}",
                    if replies_count > 0 {
                        format!(" · {replies_count}")
                    } else {
                        String::new()
                    }
                ),
                Theme::faint_style(),
            )));
            lines.push(hairline_line(w as u16));
            return Rendered {
                lines,
                highlight_from: 0,
                trailer: 5,
                title: Some(header_at),
            };
        }
    }

    Rendered {
        lines,
        highlight_from: 0,
        trailer: 1,
        title: Some(header_at),
    }
}

fn quote_lines(commit: &Commit, w: usize) -> Vec<Line<'static>> {
    if commit.is_tombstone() {
        return vec![Line::from(Span::styled("This commit was deleted.", Theme::faint_style()))];
    }
    let author = commit.author.clone().unwrap_or_default();
    let mut lines = vec![Line::from(vec![
        small_avatar(&author),
        Span::styled(truncate(author.display_name(), w / 2), Theme::bold()),
        Span::styled(
            format!(" @{} · {}", author.username, relative_time(commit.created_at)),
            Theme::dim(),
        ),
    ])];
    let mut body = text::body_lines_highlighted(commit.body.as_deref().unwrap_or(""), w, Theme::text(), &commit.highlighted_blocks());
    if body.len() > 4 {
        body.truncate(4);
        while body.last().is_some_and(|l| l.width() == 0) {
            body.pop();
        }
        body.push(Line::from(Span::styled("…", Theme::dim())));
    }
    lines.extend(body);
    lines
}

fn link_lines(preview: &LinkPreview, w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(title) = preview.title.as_deref().filter(|t| !t.trim().is_empty()) {
        lines.extend(wrap_plain(title.trim(), w, Theme::bold()).into_iter().take(2));
    }
    if let Some(description) = preview.description.as_deref().filter(|t| !t.trim().is_empty()) {
        let mut d = wrap_plain(description.trim(), w, Theme::dim());
        if d.len() > 2 {
            d.truncate(2);
        }
        lines.extend(d);
    }
    if let Some(url) = &preview.url {
        lines.push(Line::from(Span::styled(
            format!("↗\u{a0}{}", text::domain(url)),
            Theme::faint_style(),
        )));
    }
    lines
}

fn render_poll(poll: &Poll, w: usize) -> Vec<Line<'static>> {
    let bar_w = 16.min(w / 3);
    let mut lines = Vec::new();
    for option in &poll.options {
        let label = truncate(&option.label, w.saturating_sub(bar_w + 8));
        match option.percentage {
            Some(p) => {
                let filled = (p as usize * bar_w).div_ceil(100).min(bar_w);
                let color = if option.voted { Theme::primary() } else { Theme::rail() };
                lines.push(Line::from(vec![
                    Span::styled("█".repeat(filled), Style::default().fg(color)),
                    Span::styled("░".repeat(bar_w - filled), Style::default().fg(Theme::hairline())),
                    Span::styled(format!(" {p:>3}% "), Theme::bold()),
                    Span::styled(label, Theme::text()),
                    Span::styled(if option.voted { " ✓" } else { "" }, Theme::link()),
                ]));
            }
            None => lines.push(Line::from(vec![
                Span::styled("○\u{a0}", Theme::dim()),
                Span::styled(label, Theme::text()),
            ])),
        }
    }
    let mut footer = Vec::new();
    if let Some(total) = poll.total_votes {
        footer.push(format!("{total} vote{}", if total == 1 { "" } else { "s" }));
    }
    if poll.is_closed {
        footer.push("closed".into());
    } else if poll.total_votes.is_none() {
        footer.push("vote on the web to see results".into());
    }
    lines.push(Line::from(Span::styled(footer.join(" · "), Theme::faint_style())));
    lines
}

/// Draws a rounded box around lines, the way the web draws quotes and link cards.
fn boxed(inner: Vec<Line<'static>>, w: usize) -> Vec<Line<'static>> {
    let border = Style::default().fg(Theme::rail());
    let iw = w.saturating_sub(4);
    let mut out = vec![Line::from(Span::styled(format!("╭{}╮", "─".repeat(w.saturating_sub(2))), border))];
    for line in inner {
        let lw = line.width();
        let mut spans = vec![Span::styled("│ ", border)];
        spans.extend(line.spans);
        spans.push(Span::raw(" ".repeat(iw.saturating_sub(lw))));
        spans.push(Span::styled(" │", border));
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(format!("╰{}╯", "─".repeat(w.saturating_sub(2))), border)));
    out
}

/// Someone in a list of followers or following: who they are, and how they and the user are connected.
fn render_person(user: &User, w: usize, selected: bool, me: Option<&str>) -> Rendered {
    let content = w.saturating_sub(gutter_width() + 2);
    let mut chips: Vec<Span<'static>> = Vec::new();
    if user.follows_you == Some(true) && me != Some(user.username.as_str()) {
        chips.push(Span::styled(" follows you ", Style::default().fg(Theme::muted()).bg(Theme::wash())));
        chips.push(Span::raw(" "));
    }
    if me != Some(user.username.as_str()) {
        match user.is_followed {
            Some(true) => chips.push(Span::styled(" Following ", Style::default().fg(Theme::ink()).bg(Theme::wash()))),
            Some(false) => chips.push(Span::styled(
                " Follow · f ",
                Style::default().fg(Theme::on_primary()).bg(Theme::primary()),
            )),
            None => {}
        }
    }
    let chips_w: usize = chips.iter().map(|s| s.width()).sum();
    let mut head = vec![
        small_avatar(user),
        Span::styled(
            truncate(user.display_name(), content.saturating_sub(chips_w + user.username.len() + 6)),
            Theme::bold(),
        ),
        Span::styled(format!(" @{}", user.username), Theme::dim()),
    ];
    let used: usize = head.iter().map(|s| s.width()).sum();
    head.push(Span::raw(" ".repeat(content.saturating_sub(used + chips_w))));
    head.extend(chips);

    let mut lines = vec![Line::from([gutter(selected, Span::raw(" ")), head].concat())];
    if let Some(bio) = user.bio.as_deref().filter(|b| !b.trim().is_empty()) {
        let bio = truncate(bio.lines().next().unwrap_or("").trim(), content.saturating_sub(3));
        lines.push(Line::from(
            [
                gutter(selected, Span::raw(" ")),
                vec![Span::raw("   "), Span::styled(bio, Theme::dim())],
            ]
            .concat(),
        ));
    }
    lines.push(Line::default());
    lines.push(hairline_line(w as u16));
    Rendered {
        lines,
        highlight_from: 0,
        trailer: 1,
        title: None,
    }
}

fn render_notification(n: &Notification, w: usize, selected: bool) -> Rendered {
    let content = w.saturating_sub(gutter_width() + 2);
    let (icon, color, verb) = match n.kind.as_str() {
        "starred" => ("★", Theme::star(), "starred your commit"),
        "replied" => ("↩", Theme::primary(), "replied to your commit"),
        "quoted" => ("❝", Theme::lane(2), "quoted your commit"),
        "mentioned" => ("@", Theme::primary(), "mentioned you"),
        "followed" => ("+", Theme::lane(1), "followed you"),
        "approved" => ("✓", Theme::lane(1), "Your commit was approved"),
        "rejected" => ("✕", Theme::destructive(), "Your commit was rejected"),
        _ => ("•", Theme::muted(), "did something"),
    };
    let unread = n.unread_count.unwrap_or(0) > 0;
    let mut spans = gutter(
        selected,
        Span::styled(icon, Style::default().fg(color).add_modifier(Modifier::BOLD)),
    );
    if matches!(n.kind.as_str(), "approved" | "rejected") {
        spans.push(Span::styled(verb, Theme::bold()));
    } else {
        let actor = n.actor.clone().unwrap_or_default();
        if gfx::enabled() {
            spans.push(small_avatar(&actor));
        }
        spans.push(Span::styled(truncate(actor.display_name(), content / 2), Theme::bold()));
        let others = n.actors_count.unwrap_or(1).saturating_sub(1);
        if others > 0 {
            spans.push(Span::styled(
                format!(" and {others} other{}", if others == 1 { "" } else { "s" }),
                Theme::text(),
            ));
        }
        spans.push(Span::styled(format!(" {verb}"), Theme::text()));
    }
    spans.push(Span::styled(format!(" · {}", relative_time(n.created_at)), Theme::dim()));
    if unread {
        spans.push(Span::styled("  ●", Style::default().fg(Theme::primary())));
    }

    let mut lines = vec![Line::from(spans)];
    if let Some(reason) = n.reason.as_deref().filter(|r| !r.is_empty()) {
        for line in wrap_plain(reason, content, Style::default().fg(Theme::destructive())) {
            lines.push(Line::from([gutter(selected, Span::raw(" ")), line.spans].concat()));
        }
    }
    if let Some(body) = n.commit.as_ref().and_then(|c| c.body.clone()) {
        let mut body = body_lines(&body, content, Theme::dim());
        if body.len() > 2 {
            body.truncate(2);
            if let Some(last) = body.last_mut() {
                last.spans.push(Span::styled(" …", Theme::dim()));
            }
        }
        for line in body {
            lines.push(Line::from([gutter(selected, Span::raw(" ")), line.spans].concat()));
        }
    }
    lines.push(Line::default());
    lines.push(hairline_line(w as u16));
    Rendered {
        lines,
        highlight_from: 0,
        trailer: 1,
        title: None,
    }
}

// Settings ------------------------------------------------------------------------------------------------------

fn key_chip(key: &str) -> Span<'static> {
    Span::styled(
        format!(" {key}\u{a0}"),
        Style::default().fg(Theme::ink()).bg(Theme::wash()).add_modifier(Modifier::BOLD),
    )
}

fn draw_settings(f: &mut Frame, app: &App, area: Rect) {
    let w = area.width.saturating_sub(6) as usize;
    let mut lines: Vec<Line> = Vec::new();
    let section = |title: &str| Line::from(Span::styled(title.to_string(), Theme::bold()));

    lines.push(Line::default());
    lines.push(section("Account"));
    lines.push(Line::default());
    if app.signed() {
        match &app.me {
            Some(me) => lines.push(Line::from(vec![
                Span::styled("●\u{a0}", Style::default().fg(Theme::user_lane(&me.user))),
                Span::styled(me.user.display_name().to_string(), Theme::bold()),
                Span::styled(format!("  @{}", me.user.username), Theme::dim()),
            ])),
            None => lines.push(Line::from(Span::styled("Signed in.", Theme::text()))),
        }
        lines.extend(wrap_plain(
            "This terminal is one of your devices. It signs each request with its own key, so there's no password or token to leak.",
            w,
            Theme::dim(),
        ));
        if let Some(key) = crate::auth::DeviceKey::load(&config::key_path()) {
            lines.push(Line::from(Span::styled(key.fingerprint(), Theme::faint_style())));
        }
        lines.push(Line::default());
        lines.push(Line::from(vec![
            key_chip("L"),
            Span::styled("  Sign out of this terminal", Theme::text()),
        ]));

        if let Some(me) = &app.me {
            // Your color, as the web's picker offers it: six lanes, Automatic, or any hex color.
            lines.push(Line::default());
            lines.push(section("Your color"));
            lines.push(Line::default());
            let current = app.current_lane_index();
            let picked = app.lane_pick.unwrap_or(current);
            let custom = matches!(&me.preferred_lane, Some(serde_json::Value::String(_)));
            // Each color a tile, the one you're on underlined, the one you have ticked. Tiles are background color,
            // so no font draws them off size or off center.
            let mut tiles = Vec::new();
            let mut marks = Vec::new();
            for i in 0..7usize {
                let color = Theme::lane(if i < 6 { i as u8 } else { me.automatic_lane.unwrap_or(0) });
                let saved = i == current && !custom;
                let tile = Style::default().fg(Theme::shell()).bg(color).add_modifier(Modifier::BOLD);
                tiles.push(Span::styled(if saved { " ✓\u{a0}" } else { "   " }, tile));
                let under = if i == picked { "▔▔▔" } else { "   " };
                marks.push(Span::styled(under, Style::default().fg(Theme::ink())));
                let label = if i == 6 { " Automatic" } else { "" };
                let label_style = if i == picked { Theme::bold() } else { Theme::dim() };
                tiles.push(Span::styled(format!("{label}  "), label_style));
                marks.push(Span::raw(" ".repeat(width(label) + 2)));
            }
            if custom {
                tiles.push(Span::styled("custom ", Theme::dim()));
                tiles.push(Span::styled(
                    " ✓\u{a0}",
                    Style::default().fg(Theme::shell()).bg(Theme::user_lane(&me.user)),
                ));
            }
            lines.push(Line::from(tiles));
            lines.push(Line::from(marks));
            let hint = if app.lane_pick.is_some_and(|p| p != current) {
                "↵\u{a0}saves it · esc keeps yours"
            } else {
                "←\u{a0}→\u{a0}pick a lane · # any hex color"
            };
            lines.push(Line::from(Span::styled(hint, Theme::faint_style())));

            lines.push(Line::default());
            lines.push(Line::from(vec![
                key_chip("P"),
                Span::styled("  Pastéis mode  ", Theme::text()),
                Span::styled(
                    if me.pasteis_mode { "on" } else { "off" },
                    if me.pasteis_mode {
                        Style::default().fg(Theme::lane(5))
                    } else {
                        Theme::dim()
                    },
                ),
            ]));
            lines.extend(text::wrap_indented(
                "A pastel de nata button on every commit, ten to give a day.",
                w,
                Theme::faint_style(),
                5,
            ));
            lines.push(Line::default());
            let synced = me
                .github_synced_at
                .map(|t| format!("  last synced {}", relative_time(Some(t))))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                key_chip("G"),
                Span::styled("  Sync your profile from GitHub", Theme::text()),
                Span::styled(synced, Theme::faint_style()),
            ]));
        }
    } else {
        lines.extend(wrap_plain(
            "You're reading as a guest. Sign in to get the Following, Latest and Trending logs, notifications, replies, stars and quotes.",
            w,
            Theme::dim(),
        ));
        lines.push(Line::default());
        lines.push(Line::from(vec![
            key_chip("s"),
            Span::styled("  Sign in with your browser", Theme::text()),
        ]));
        lines.extend(text::wrap_indented(
            "Approves this terminal as a device, the way the mobile app signs in.",
            w,
            Theme::faint_style(),
            5,
        ));
        lines.push(Line::default());
        let username = app
            .config
            .username
            .clone()
            .map(|u| format!("@{u}"))
            .unwrap_or_else(|| "not set".into());
        lines.push(Line::from(vec![
            key_chip("u"),
            Span::styled("  GitHub username  ", Theme::text()),
            Span::styled(username, Theme::link()),
        ]));
        lines.extend(text::wrap_indented(
            "Without signing in, your Following log is built from the public commits of the people you follow.",
            w,
            Theme::faint_style(),
            5,
        ));
        if app.config.username.is_some() {
            lines.push(Line::default());
            lines.push(Line::from(vec![key_chip("⏎"), Span::styled("  Go to your log", Theme::text())]));
        }
    }

    if let Some(sign_in) = &app.sign_in {
        lines.push(Line::default());
        lines.push(section("Signing in"));
        lines.push(Line::default());
        lines.extend(wrap_plain(
            "1. Approve this terminal on the page we opened. Not there?",
            w,
            Theme::text(),
        ));
        lines.push(Line::from(vec![
            Span::raw("   "),
            key_chip("w"),
            Span::styled(" opens it again   ", Theme::dim()),
            key_chip("y"),
            Span::styled(" copies the link", Theme::dim()),
        ]));
        if sign_in.handler.is_some() {
            lines.extend(wrap_plain(
                "2. When your browser asks to open \"devnull sign-in\", allow it. This finishes on its own.",
                w,
                Theme::text(),
            ));
            lines.extend(wrap_plain(
                "   Didn't ask? Copy the devnull:// link it was sent to (DevTools →\u{a0}Network shows it) and paste it.",
                w,
                Theme::dim(),
            ));
        } else {
            lines.extend(wrap_plain("2. Your browser is then sent to a devnull:// link, meant for the app. Copy it from the address bar (or DevTools →\u{a0}Network) and paste it here.", w, Theme::text()));
        }
        lines.push(Line::from(vec![
            Span::raw("   "),
            key_chip("v"),
            Span::styled(" paste the link", Theme::dim()),
        ]));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            truncate(&sign_in.authorization.url, w),
            Theme::faint_style(),
        )));
    }

    lines.push(Line::default());
    lines.push(section("Committing"));
    lines.push(Line::default());
    if app.signed() {
        lines.extend(wrap_plain("Commits, replies and quotes are signed by this terminal's device key. They go through moderation like any other, so a new one may be pending for a moment.", w, Theme::dim()));
    } else {
        lines.extend(wrap_plain("Without signing in, commits and replies are signed with the SSH key on your GitHub account, through ssh-keygen, the same way `curl -fsSL dev-null.sh/commit | bash` does it. No login, no token, and your private key never leaves ssh-keygen or your agent.", w, Theme::dim()));
    }

    lines.push(Line::default());
    lines.push(section("Look"));
    lines.push(Line::default());
    let shown = match (app.config.icons().as_str(), icons::nerd()) {
        ("auto", true) => "auto · Nerd Font, your terminal has them",
        ("auto", false) => "auto · plain symbols",
        ("nerd", _) => "Nerd Font",
        _ => "plain symbols",
    };
    let icon = icons::get();
    lines.push(Line::from(vec![
        key_chip("i"),
        Span::styled("  Icons  ", Theme::text()),
        Span::styled(shown, Theme::link()),
        Span::styled(
            format!(
                "  {}{g}{}{g}{}{g}{}",
                icon.log,
                icon.notifications,
                icon.remotes,
                icon.profile,
                g = icons::gap()
            ),
            Theme::dim(),
        ),
    ]));
    if config::THEME_PICKER {
        lines.push(Line::default());
        let shown = match (app.config.theme().as_str(), app.detected_light) {
            ("auto", Some(true)) => "auto · light, like your terminal".to_string(),
            ("auto", Some(false)) => "auto · dark, like your terminal".to_string(),
            ("auto", None) => "auto · dark (your terminal didn't say)".to_string(),
            _ => Theme::name(),
        };
        lines.push(Line::from(vec![
            key_chip("t"),
            Span::styled("  Theme  ", Theme::text()),
            Span::styled(shown, Theme::link()),
            Span::styled("  T goes back", Theme::faint_style()),
        ]));
        lines.extend(text::wrap_indented(
        &format!(
            "Auto follows your terminal's background. Your own themes go in {}, as TOML files like the ones in the repo's themes/ folder.",
            text::tilde(&crate::theme::custom_dir())
        ),
        w,
        Theme::faint_style(),
        5,
    ));
    }
    lines.push(Line::default());
    lines.push(section("Server"));
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled(app.api.base.clone(), Theme::link()),
        Span::styled("   DEVNULL_URL changes it", Theme::faint_style()),
    ]));
    lines.push(Line::from(Span::styled(
        format!("Config in {}", config::dir().display()),
        Theme::faint_style(),
    )));

    let p = Paragraph::new(lines);
    f.render_widget(
        p,
        Rect {
            x: area.x + 3,
            width: area.width.saturating_sub(5),
            ..area
        },
    );
}

// Right panel ---------------------------------------------------------------------------------------------------

fn draw_right(f: &mut Frame, app: &App, area: Rect) {
    let x = area.x + 2;
    let w = area.width.saturating_sub(3);
    let buf = f.buffer_mut();

    // Search
    let search = Rect {
        x,
        y: area.y + 1,
        width: w,
        height: 3,
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::borders())
        .border_style(Style::default().fg(Theme::hairline()))
        .render_ref(search, buf);
    buf.set_string(
        x + 2,
        search.y + 1,
        format!("{}{} Search", icons::get().search, icons::gap()),
        Theme::dim(),
    );
    buf.set_string(
        x + w - 4,
        search.y + 1,
        " / ",
        Style::default().fg(Theme::muted()).bg(Theme::wash()),
    );

    let mut y = area.y + 5;
    match &app.me {
        Some(me) => {
            let user = &me.user;
            // A large name takes two rows, so the avatar grows to three to sit beside it and the handle.
            let name = truncate(user.display_name(), (w as usize).saturating_sub(8) * 2 / 3);
            let large = big::rows(&name, Size::Large, Theme::bold());
            let (cols, rows) = if large.is_some() { (6, 3) } else { (4, 2) };
            let avatar: Vec<Vec<Span>> = match (0..rows).map(|row| avatar_row(user, cols, rows, row)).collect::<Option<Vec<_>>>() {
                Some(rows) => rows.into_iter().map(|row| vec![row, Span::raw(" ")]).collect(),
                None => {
                    let mut lead = vec![vec![Span::styled("⦿\u{a0}", Style::default().fg(Theme::user_lane(user)))]];
                    lead.resize(rows as usize, vec![Span::raw("  ")]);
                    lead
                }
            };
            let mut text: Vec<Vec<Span>> = match large {
                Some((top, bottom, _)) => vec![vec![top], vec![bottom]],
                None => vec![vec![Span::styled(truncate(user.display_name(), w as usize - 6), Theme::bold())]],
            };
            text.push(vec![
                Span::styled("●\u{a0}", Style::default().fg(Theme::lane(1))),
                Span::styled(format!("@{}", user.username), Theme::dim()),
            ]);
            for (lead, rest) in avatar.into_iter().zip(text) {
                buf.set_line(x, y, &Line::from([lead, rest].concat()), w);
                y += 1;
            }
            y += 1;
            let mut row: Vec<Span> = Vec::new();
            for (n, label) in [
                (user.commits_count, "commits"),
                (user.following_count, "following"),
                (user.followers_count, "followers"),
            ] {
                let Some(n) = n else { continue };
                let group = [
                    Span::styled(count(n), Theme::bold()),
                    Span::styled(format!(" {label}  "), Theme::dim()),
                ];
                let used: usize = row.iter().map(|s| s.width()).sum();
                if used + group.iter().map(|s| s.width()).sum::<usize>() > w as usize + 2 {
                    buf.set_line(x, y, &Line::from(std::mem::take(&mut row)), w);
                    y += 1;
                }
                row.extend(group);
            }
            buf.set_line(x, y, &Line::from(row), w);
            y += 2;
            if let Some(bio) = user.bio.as_deref().filter(|b| !b.trim().is_empty()) {
                for line in wrap_plain(bio.trim(), w as usize, Theme::dim()).into_iter().take(3) {
                    buf.set_line(x, y, &line, w);
                    y += 1;
                }
                y += 1;
            }
            if me.pasteis_mode {
                let left = me.pasteis_remaining.unwrap_or(0);
                buf.set_line(
                    x,
                    y,
                    &Line::from(vec![
                        Span::styled("⊚\u{a0}", Style::default().fg(Theme::lane(5))),
                        Span::styled(format!("{left} pastéis left today"), Theme::dim()),
                    ]),
                    w,
                );
                y += 2;
            }
        }
        None => {
            let lines = [
                Line::from(Span::styled("Not signed in", Theme::bold())),
                Line::from(Span::styled(
                    app.config
                        .username
                        .clone()
                        .map(|u| format!("Reading as @{u}"))
                        .unwrap_or_else(|| "Reading as a guest".into()),
                    Theme::dim(),
                )),
                Line::default(),
                Line::from(vec![key_chip("g s"), Span::styled(" to sign in", Theme::dim())]),
            ];
            for line in lines {
                buf.set_line(x, y, &line, w);
                y += 1;
            }
            y += 1;
        }
    }

    hairline(buf, x, y, w);
    y += 2;
    buf.set_string(x, y, "Shortcuts", Theme::bold());
    y += 2;
    let shortcuts = [
        ("j k", "move"),
        ("⏎", "open"),
        ("r", "reply"),
        ("q", "quote"),
        ("s", "star"),
        ("c", "commit"),
        ("/", "go to"),
    ];
    for (key, label) in shortcuts {
        if y + 4 >= area.y + area.height {
            break;
        }
        buf.set_line(
            x,
            y,
            &Line::from(vec![Span::styled(format!("{label:<10}"), Theme::dim()), key_chip(key)]),
            w,
        );
        y += 1;
    }

    let bottom = area.y + area.height - 2;
    buf.set_line(
        x,
        bottom - 1,
        &Line::from(vec![Span::styled("Keyboard shortcuts ", Theme::dim()), key_chip("?")]),
        w,
    );
    buf.set_string(x, bottom, text::domain(&app.api.base), Theme::faint_style());
}

trait RenderRef {
    fn render_ref(self, area: Rect, buf: &mut Buffer);
}

impl RenderRef for Block<'_> {
    fn render_ref(self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::Widget;
        self.render(area, buf);
    }
}

// Status bar -----------------------------------------------------------------------------------------------------

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let buf = f.buffer_mut();
    let spinner = SPINNER[app.tick % SPINNER.len()];

    let left: Line = if let Some(busy) = &app.busy {
        Line::from(vec![
            Span::styled(format!(" {spinner} "), Theme::link()),
            Span::styled(busy.clone(), Theme::text()),
        ])
    } else if let Some((message, error, _)) = &app.toast {
        let color = if *error { Theme::destructive() } else { Theme::lane(1) };
        Line::from(vec![
            Span::styled(if *error { " ✕\u{a0}" } else { " ✓\u{a0}" }, Style::default().fg(color)),
            Span::styled(message.clone(), Theme::text()),
        ])
    } else if app.pending_g {
        Line::from(vec![
            Span::styled(" g ", Style::default().fg(Theme::on_primary()).bg(Theme::primary())),
            Span::styled(
                "  l log  e trending  f following  n notifications  r remotes  p profile  b starred  s settings  g top",
                Theme::dim(),
            ),
        ])
    } else {
        let hints: &[(&str, &str)] = match app.screen() {
            Screen::Settings => &[("g l", "log"), ("?", "help"), ("Q", "quit")],
            Screen::Remotes if app.remotes.searching => &[("⏎", "search"), ("esc", "clear")],
            Screen::Remotes => &[
                ("j/k", "move"),
                ("⏎", "open"),
                ("n", "new"),
                ("x", "stash"),
                ("/", "search"),
                ("S", "share pulls"),
                ("⇥", "tabs"),
                ("esc", "back"),
            ],
            Screen::Remote(handle) if app.remotes.chats.get(handle).is_some_and(|c| c.typing) => {
                &[("⏎", "push"), ("alt ⏎", "new line"), ("esc", "stop")]
            }
            Screen::Remote(_) => &[
                ("j/k", "move"),
                ("i", "write"),
                ("r", "reply"),
                ("e", "react"),
                ("v", "images"),
                ("x", "stash"),
                ("esc", "back"),
            ],
            Screen::Profile(..) => &[
                ("j/k", "move"),
                ("⏎", "open"),
                ("f", "follow"),
                ("M", "remote"),
                ("m", "mute"),
                ("⇥", "tabs"),
                ("esc", "back"),
            ],
            Screen::Thread(_) => &[("j/k", "move"), ("⏎", "reply"), ("s", "star"), ("a", "author"), ("esc", "back")],
            _ => &[
                ("j/k", "move"),
                ("⏎", "open"),
                ("r", "reply"),
                ("c", "commit"),
                ("⇥", "tab"),
                ("?", "help"),
            ],
        };
        let mut spans = vec![Span::raw(" ")];
        for (key, label) in hints {
            spans.push(Span::styled(key.to_string(), Theme::text()));
            spans.push(Span::styled(format!("\u{a0}{label}   "), Theme::faint_style()));
        }
        Line::from(spans)
    };
    buf.set_line(area.x, area.y, &left, area.width);
}

// Overlays ------------------------------------------------------------------------------------------------------

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn modal(f: &mut Frame, rect: Rect, title: &str) -> Rect {
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::borders())
        .border_style(Style::default().fg(Theme::rail()))
        .title(Span::styled(format!(" {title} "), Theme::bold()))
        .style(Style::default().bg(Theme::popover()));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    // With a theme's title bars, the top edge becomes the window's, with its close button.
    if let Some(bar) = Theme::title_bar().filter(|_| rect.width > 8) {
        let buf = f.buffer_mut();
        let (start, end) = (rect.x + 1, rect.x + rect.width - 1);
        for x in start..end {
            let t = (x - start) as f32 / (end - start) as f32;
            buf[(x, rect.y)]
                .set_symbol(" ")
                .set_bg(rgb(theme::fade(bar.active.0, bar.active.1, t)));
        }
        let ink = Style::default().fg(rgb(bar.active_ink)).add_modifier(Modifier::BOLD);
        buf.set_stringn(start + 1, rect.y, title, (end - start).saturating_sub(5) as usize, ink);
        buf[(end - 2, rect.y)]
            .set_symbol("×")
            .set_style(Style::default().fg(Theme::ink()).bg(Theme::popover()).add_modifier(Modifier::BOLD));
    }
    Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    }
}

pub const SHORTCUTS: &[(&str, &[&str])] = &[
    ("Next commit", &["j"]),
    ("Previous commit", &["k"]),
    ("Copy link to the focused commit", &["y"]),
    ("Open the focused commit", &["⏎"]),
    ("Reply to commit", &["r"]),
    ("Quote commit", &["q"]),
    ("Show or hide replies", &["o"]),
    ("Write a commit", &["c"]),
    ("Go to a developer or commit", &["⌃", "K"]),
    ("Go to a developer or commit", &["/"]),
    ("Commit (in the composer)", &["⌃", "S"]),
    ("Preview (in the composer)", &["⌃", "P"]),
    ("Go to your log", &["g", "l"]),
    ("Go to trending", &["g", "e"]),
    ("Go to your profile", &["g", "p"]),
    ("Go to settings", &["g", "s"]),
    ("Go to Remotes (messages)", &["g", "r"]),
    ("Show this list", &["?"]),
];

pub const TERMINAL_SHORTCUTS: &[(&str, &[&str])] = &[
    ("Star", &["s"]),
    ("Give a pastel de nata", &["p"]),
    ("View images full size", &["v"]),
    ("Open the author's profile", &["a"]),
    ("Open in your browser", &["w"]),
    ("Report a commit", &["!"]),
    ("Delete your commit", &["D"]),
    ("Follow or unfollow", &["f"]),
    ("Start a remote (on a profile)", &["M"]),
    ("Mute / block (on a profile)", &["m", "B"]),
    ("Achievements (on a profile)", &["A"]),
    ("Next tab or filter", &["⇥"]),
    ("Notifications", &["g", "n"]),
    ("Starred", &["g", "b"]),
    ("Remotes: new, stash, share pulls", &["n", "x", "S"]),
    ("In a conversation: write, reply, react", &["i", "r", "e"]),
    ("Top / bottom", &["g", "g"]),
    ("Refresh", &["R"]),
    ("Back", &["esc"]),
    ("Quit", &["⌃", "C"]),
];

pub const COMPOSER_SHORTCUTS: &[(&str, &[&str])] = &[
    ("Complete an @mention", &["⇥"]),
    ("Attach the clipboard's image", &["⌃", "V"]),
    ("Remove the last image", &["⌃", "X"]),
];

fn draw_help(f: &mut Frame, app: &mut App, area: Rect) {
    let rect = centered(area, 62, 48);
    let inner = modal(f, rect, "Keyboard shortcuts");
    let mut lines = vec![
        Line::from(Span::styled("They work anywhere except while you're typing.", Theme::dim())),
        Line::default(),
    ];
    let row = |label: &str, keys: &[&str], w: usize| -> Line<'static> {
        let mut chips: Vec<Span> = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                chips.push(Span::raw(" "));
            }
            chips.push(key_chip(key));
        }
        let chips_w: usize = chips.iter().map(|s| s.width()).sum();
        let mut spans = vec![Span::styled(label.to_string(), Theme::text())];
        spans.push(Span::raw(" ".repeat(w.saturating_sub(width(label) + chips_w))));
        spans.extend(chips);
        Line::from(spans)
    };
    let w = inner.width as usize;
    let roomy = inner.height as usize >= (SHORTCUTS.len() + TERMINAL_SHORTCUTS.len()) * 2 + COMPOSER_SHORTCUTS.len() + 9;
    for (label, keys) in SHORTCUTS {
        lines.push(row(label, keys, w));
        if roomy {
            lines.push(Line::from(Span::styled("─".repeat(w), Style::default().fg(Theme::hairline()))));
        }
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled("In the composer", Theme::bold())));
    lines.push(Line::default());
    for (label, keys) in COMPOSER_SHORTCUTS {
        lines.push(row(label, keys, w));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled("In the terminal", Theme::bold())));
    lines.push(Line::default());
    for (label, keys) in TERMINAL_SHORTCUTS {
        lines.push(row(label, keys, w));
        if roomy {
            lines.push(Line::from(Span::styled("─".repeat(w), Style::default().fg(Theme::hairline()))));
        }
    }
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    app.help_scroll = app.help_scroll.min(max_scroll);
    f.render_widget(Paragraph::new(lines).scroll((app.help_scroll as u16, 0)), inner);
}

/// A commit's image, as large as the terminal allows.
fn draw_viewer(f: &mut Frame, viewer: &Viewer, area: Rect) {
    f.render_widget(Clear, area);
    f.buffer_mut().set_style(area, Style::default().bg(Theme::shell()));
    let image = &viewer.images[viewer.index];
    let (max_cols, max_rows) = (area.width.saturating_sub(4), area.height.saturating_sub(4));
    let (cols, rows) = gfx::fit(image.width.unwrap_or(4), image.height.unwrap_or(3), max_cols, max_rows);
    let key = gfx::Key {
        url: image.url.clone().unwrap_or_default(),
        cols,
        rows,
        ring: None,
    };
    let x = area.x + (area.width - cols) / 2;
    let y = area.y + 1 + (max_rows - rows) / 2;
    let buf = f.buffer_mut();
    for row in 0..rows {
        match gfx::row(&key, row) {
            Some(span) => {
                buf.set_line(x, y + row, &Line::from(span), cols);
            }
            None if row == rows / 2 => {
                let label = "Loading…";
                buf.set_string(area.x + (area.width - label.len() as u16) / 2, y + row, label, Theme::dim());
            }
            None => {}
        }
    }
    let size = match (image.width, image.height) {
        (Some(w), Some(h)) => format!(" · {w}×{h}"),
        _ => String::new(),
    };
    let caption = Line::from(vec![
        Span::styled(format!("@{}", viewer.author), Theme::bold()),
        Span::styled(format!("  {}/{}{size}", viewer.index + 1, viewer.images.len()), Theme::dim()),
        Span::styled(
            if viewer.images.len() > 1 { "   ←\u{a0}→\u{a0}next" } else { "" },
            Theme::faint_style(),
        ),
        Span::styled("   w browser   esc close", Theme::faint_style()),
    ]);
    let cw = caption.width() as u16;
    buf.set_line(
        area.x + area.width.saturating_sub(cw) / 2,
        area.y + area.height - 2,
        &caption,
        area.width,
    );
}

/// Every badge, with how far along it is, and the XP to the next Unix rank.
fn draw_achievements(f: &mut Frame, profile: &Profile, area: Rect) {
    let Some(a) = &profile.achievements else { return };
    let lane = Theme::user_lane_hex(&profile.data);
    let rect = centered(area, 70, a.badges.len() as u16 + 9);
    let inner = modal(f, rect, &format!("@{}'s achievements", profile.data.username));
    let w = inner.width as usize;

    let mut lines = Vec::new();
    let next = a
        .next_level_xp
        .map(|n| format!(" / {}", count(n.max(0) as u64)))
        .unwrap_or_default();
    lines.push(Line::from(vec![
        Span::styled(
            a.rank.clone(),
            Style::default().fg(crate::theme::rgb(lane)).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  level {}  ·  {}{} XP", a.level, count(a.xp.max(0) as u64), next),
            Theme::dim(),
        ),
    ]));
    let bar_w = w.saturating_sub(2).min(60);
    let filled = (a.level_progress.min(100) as usize * bar_w) / 100;
    lines.push(Line::from(vec![
        Span::styled("━".repeat(filled), Style::default().fg(crate::theme::rgb(lane))),
        Span::styled("━".repeat(bar_w - filled), Style::default().fg(Theme::hairline())),
    ]));
    lines.push(Line::default());

    for badge in &a.badges {
        let (mark, style) = if badge.unlocked {
            ("✓", Style::default().fg(crate::theme::rgb(lane)).add_modifier(Modifier::BOLD))
        } else {
            ("○", Theme::faint_style())
        };
        let progress = if badge.unlocked || badge.target <= 1 {
            String::new()
        } else {
            format!(" {}/{}", badge.progress, badge.target)
        };
        let head = format!("{:<16}", badge.name);
        let desc_w = w.saturating_sub(head.len() + progress.len() + 4);
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} "), style),
            Span::styled(head, if badge.unlocked { Theme::bold() } else { Theme::dim() }),
            Span::styled(
                truncate(&badge.description, desc_w),
                if badge.unlocked { Theme::text() } else { Theme::faint_style() },
            ),
            Span::styled(progress, Theme::faint_style()),
        ]));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![key_chip("any key"), Span::styled(" close", Theme::dim())]));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_confirm(f: &mut Frame, confirm: &Confirm, area: Rect) {
    if let Confirm::Report(commit) = confirm {
        let rect = centered(area, 56, 12);
        let inner = modal(f, rect, "Report commit");
        let who = commit.author.as_ref().map(|a| format!("@{}", a.username)).unwrap_or_default();
        let mut lines = wrap_plain(
            &format!(
                "Why should moderators look at this commit by {who}? They'll see your report privately, and the commit stays up until they decide."
            ),
            inner.width as usize,
            Theme::text(),
        );
        lines.push(Line::default());
        for (i, (_, label)) in REPORT_REASONS.iter().enumerate() {
            lines.push(Line::from(vec![
                key_chip(&(i + 1).to_string()),
                Span::styled(format!("  {label}"), Theme::text()),
            ]));
        }
        lines.push(Line::default());
        lines.push(Line::from(vec![key_chip("any other key"), Span::styled(" cancel", Theme::dim())]));
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }
    let mute_text;
    let block_text;
    let (title, message) = match confirm {
        Confirm::Report(_) => unreachable!(),
        Confirm::Mute(username, true) => {
            mute_text = format!("Mute @{username}? Their commits leave your logs. They aren't told, and can still see yours.");
            ("Mute", mute_text.as_str())
        }
        Confirm::Mute(username, false) => {
            mute_text = format!("Unmute @{username}? Their commits come back to your logs.");
            ("Unmute", mute_text.as_str())
        }
        Confirm::Block(username, on) => {
            block_text = if *on {
                format!(
                    "Block @{username}? Neither of you will see the other's commits, and you'll stop following each other. Unblocking doesn't bring the follows back."
                )
            } else {
                format!("Unblock @{username}?")
            };
            (if *on { "Block" } else { "Unblock" }, block_text.as_str())
        }
        Confirm::Delete(_) => ("Delete commit", "Delete this commit? It goes to /dev/null, for real this time."),
        Confirm::SignOut => (
            "Sign out",
            "Sign out of this terminal? Its device key is deleted here; revoke it under Settings →\u{a0}Devices on the web too.",
        ),
    };
    let rect = centered(area, 56, 8);
    let inner = modal(f, rect, title);
    let mut lines = wrap_plain(message, inner.width as usize, Theme::text());
    lines.push(Line::default());
    lines.push(Line::from(vec![
        key_chip("y"),
        Span::styled(" yes   ", Theme::dim()),
        key_chip("any key"),
        Span::styled(" no", Theme::dim()),
    ]));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let Some(input) = &app.input else { return };
    let (title, hint, placeholder) = match input.kind {
        InputKind::Go => ("Go to", "A @username, a profile or commit link, or a commit id.", "@nunomaduro"),
        InputKind::Username => (
            "Your GitHub username",
            "Used to build your Following log without signing in.",
            "octocat",
        ),
        InputKind::LaneColor => (
            "Your color",
            "Any color, as six hex digits. Lanes look best on the dark theme, but it's yours.",
            "#3bc9db",
        ),
        InputKind::SignInCode if app.sign_in.as_ref().is_some_and(|s| s.handler.is_some()) => (
            "Sign in",
            "Approve this terminal in your browser. When it asks to open \"devnull sign-in\", allow it, and you're in. If it doesn't ask, paste the devnull:// link here.",
            "devnull://auth/callback?code=…",
        ),
        InputKind::SignInCode => (
            "Sign in",
            "Approve this terminal in your browser. You're then sent to a devnull://auth/callback?code=… link: paste it here.",
            "devnull://auth/callback?code=…",
        ),
    };
    let rect = centered(area, 72, if input.kind == InputKind::SignInCode { 12 } else { 9 });
    let inner = modal(f, rect, title);
    let w = inner.width as usize;

    let mut lines = wrap_plain(hint, w, Theme::dim());
    if input.kind == InputKind::SignInCode {
        lines.push(Line::from(vec![
            key_chip("esc"),
            Span::styled(" then w reopens the page, y copies its link", Theme::faint_style()),
        ]));
    }
    lines.push(Line::default());
    let field_y = inner.y + lines.len() as u16;
    let text: String = input.text.iter().collect();
    let before: String = input.text[..input.cursor].iter().collect();
    let (shown, cursor_x) = {
        let field_w = w.saturating_sub(4);
        let bw = width(&before);
        if bw < field_w {
            (text.clone(), bw)
        } else {
            let skip = bw - field_w + 1;
            let s: String = text.chars().skip(skip).collect();
            (s, field_w - 1)
        }
    };
    let field = if text.is_empty() {
        Span::styled(placeholder, Theme::faint_style())
    } else {
        Span::styled(shown, Theme::text())
    };
    lines.push(Line::from(vec![Span::styled("›\u{a0}", Theme::link()), field]));
    lines.push(Line::default());
    if let Some(error) = &input.error {
        lines.extend(wrap_plain(error, w, Style::default().fg(Theme::destructive())));
    } else if app.sign_in.as_ref().is_some_and(|s| s.redeeming) && input.kind == InputKind::SignInCode {
        lines.push(Line::from(Span::styled("Signing in…", Theme::link())));
    } else {
        lines.push(Line::from(vec![
            key_chip("⏎"),
            Span::styled(" go   ", Theme::faint_style()),
            key_chip("esc"),
            Span::styled(" cancel", Theme::faint_style()),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
    f.set_cursor_position((inner.x + 2 + cursor_x as u16, field_y));
}

fn draw_composer(f: &mut Frame, app: &App, _center: Rect, area: Rect) {
    let Some(composer) = &app.composer else { return };
    let w = 76.min(area.width.saturating_sub(4));
    let (title, context): (String, Option<&Commit>) = match &composer.mode {
        ComposeMode::New => ("New commit".into(), None),
        ComposeMode::Reply(c) => (
            format!("Reply to @{}", c.author.as_ref().map(|a| a.username.as_str()).unwrap_or("?")),
            Some(c),
        ),
        ComposeMode::Quote(c) => (
            format!("Quote @{}", c.author.as_ref().map(|a| a.username.as_str()).unwrap_or("?")),
            Some(c),
        ),
    };

    let iw = w.saturating_sub(4) as usize;
    let context_lines: Vec<Line> = context.map(|c| boxed(quote_lines(c, iw.saturating_sub(4)), iw)).unwrap_or_default();
    let text_h = 8u16;
    let image_rows = if composer.images.is_empty() {
        0
    } else {
        composer.images.len() as u16 + 1
    };
    // An error gets as many lines as it needs, above the footer, rather than being cut short.
    let error_lines: Vec<Line> = composer
        .error
        .as_deref()
        .map(|e| wrap_plain(e, w.saturating_sub(6) as usize, Style::default().fg(Theme::destructive())))
        .unwrap_or_default()
        .into_iter()
        .take(5)
        .collect();
    let error_rows = if error_lines.is_empty() { 0 } else { error_lines.len() as u16 + 1 };
    let h = (context_lines.len() as u16 + text_h + image_rows + error_rows + 7).min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    let inner = modal(f, rect, &title);
    let buf = f.buffer_mut();

    let mut y = inner.y;
    let lane = app.me.as_ref().map(|m| Theme::user_lane(&m.user)).unwrap_or(Theme::rail());

    // A reply shows what it replies to above; a quote shows what it quotes below, as the web does.
    let reply = matches!(composer.mode, ComposeMode::Reply(_));
    if reply {
        for line in &context_lines {
            buf.set_line(inner.x, y, line, inner.width);
            y += 1;
        }
        buf.set_string(inner.x + 1, y, "│", Style::default().fg(lane));
        y += 1;
    }

    let text_area = Rect {
        x: inner.x + 2,
        y,
        width: inner.width.saturating_sub(2),
        height: text_h.min((inner.y + inner.height).saturating_sub(y + 3 + error_rows)),
    };
    buf.set_string(inner.x, y, "●", Style::default().fg(lane));

    let mut cursor_pos = (text_area.x, text_area.y);
    if composer.preview {
        let lines = body_lines(&composer.body(), text_area.width as usize, Theme::text());
        for (i, line) in lines.iter().take(text_area.height as usize).enumerate() {
            buf.set_line(text_area.x, text_area.y + i as u16, line, text_area.width);
        }
    } else if composer.text.is_empty() {
        buf.set_string(
            text_area.x,
            text_area.y,
            if reply { "Commit your reply" } else { "What did you ship?" },
            Theme::faint_style(),
        );
    } else {
        // Wraps by character, so the cursor lands where the text does.
        let mut rows: Vec<(String, usize)> = vec![(String::new(), 0)];
        let tw = text_area.width as usize;
        let mut cursor_row_col = (0usize, 0usize);
        for (i, ch) in composer.text.iter().enumerate() {
            if i == composer.cursor {
                let last = rows.len() - 1;
                cursor_row_col = (last, rows[last].1);
            }
            if *ch == '\n' {
                rows.push((String::new(), 0));
                continue;
            }
            let cw = unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(0);
            if rows.last().unwrap().1 + cw > tw {
                rows.push((String::new(), 0));
            }
            let last = rows.last_mut().unwrap();
            last.0.push(*ch);
            last.1 += cw;
        }
        if composer.cursor >= composer.text.len() {
            let last = rows.len() - 1;
            cursor_row_col = (last, rows[last].1);
            if rows[last].1 >= tw {
                cursor_row_col = (last + 1, 0);
            }
        }
        let visible = text_area.height as usize;
        let first = (cursor_row_col.0 + 1).saturating_sub(visible);
        for (i, (row, _)) in rows.iter().skip(first).take(visible).enumerate() {
            buf.set_string(text_area.x, text_area.y + i as u16, row, Theme::text());
        }
        cursor_pos = (
            text_area.x + cursor_row_col.1 as u16,
            text_area.y + (cursor_row_col.0 - first) as u16,
        );
    }
    y = text_area.y + text_area.height;

    // Attached images, in the order they'll show.
    if !composer.images.is_empty() {
        y += 1;
        for (i, image) in composer.images.iter().enumerate() {
            if y >= inner.y + inner.height - 2 {
                break;
            }
            let line = Line::from(vec![
                Span::styled("  ⊞\u{a0}", Style::default().fg(lane)),
                Span::styled(truncate(&image.name, inner.width as usize / 2), Theme::text()),
                Span::styled(format!("  {}×{} · {}", image.width, image.height, image.size_label()), Theme::dim()),
                Span::styled(
                    if i + 1 == composer.images.len() { "   ⌃X removes" } else { "" },
                    Theme::faint_style(),
                ),
            ]);
            buf.set_line(inner.x, y, &line, inner.width);
            y += 1;
        }
    }

    if !reply {
        for line in &context_lines {
            if y >= inner.y + inner.height - 2 {
                break;
            }
            buf.set_line(inner.x, y, line, inner.width);
            y += 1;
        }
    }

    // Footer: the error or what's going on, the count, and the button.
    let fy = inner.y + inner.height - 1;
    for (i, line) in error_lines.iter().enumerate() {
        buf.set_line(inner.x, fy - 1 - error_rows + i as u16, line, inner.width);
    }
    hairline(buf, inner.x, fy - 1, inner.width);
    let n = composer.text.iter().filter(|c| !c.is_whitespace() || **c == ' ').count();
    let over = composer.body().chars().count() > MAX_BODY;
    let status: Line = if composer.sending {
        Line::from(Span::styled(
            app.busy.clone().unwrap_or_else(|| "Committing…".into()),
            Theme::link(),
        ))
    } else if composer.preview {
        Line::from(vec![
            Span::styled("Preview · ", Theme::link()),
            Span::styled("⌃P to edit", Theme::faint_style()),
        ])
    } else {
        let signing = if !app.signed() || !composer.images.is_empty() {
            " · signed with your GitHub SSH key"
        } else {
            ""
        };
        Line::from(Span::styled(
            format!("⌃S commit · ⌃V image · esc close{signing}"),
            Theme::faint_style(),
        ))
    };
    buf.set_line(inner.x, fy, &status, inner.width.saturating_sub(22));

    let counter = format!("{n}/{MAX_BODY}");
    let button = " Commit ";
    let bx = inner.x + inner.width - width(button) as u16;
    let empty = composer.body().is_empty();
    buf.set_string(
        bx - counter.len() as u16 - 2,
        fy,
        &counter,
        if over {
            Style::default().fg(Theme::destructive())
        } else {
            Theme::faint_style()
        },
    );
    buf.set_string(
        bx,
        fy,
        button,
        if empty || over || composer.sending {
            Style::default().fg(Theme::muted()).bg(Theme::primary_muted())
        } else {
            Style::default()
                .fg(Theme::on_primary())
                .bg(Theme::primary())
                .add_modifier(Modifier::BOLD)
        },
    );

    if !composer.preview && !composer.sending {
        f.set_cursor_position(cursor_pos);

        // @mention suggestions, under the cursor.
        if let Some((_, prefix)) = composer.mention() {
            let candidates = app.mention_candidates(&prefix);
            if !candidates.is_empty() {
                let w = 44.min(area.width.saturating_sub(4));
                let h = candidates.len() as u16 + 2;
                let x = cursor_pos.0.saturating_sub(prefix.len() as u16 + 1).min(area.x + area.width - w);
                let y = if cursor_pos.1 + 1 + h <= area.y + area.height {
                    cursor_pos.1 + 1
                } else {
                    cursor_pos.1.saturating_sub(h)
                };
                let rect = Rect { x, y, width: w, height: h };
                f.render_widget(Clear, rect);
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(Theme::borders())
                    .border_style(Style::default().fg(Theme::rail()))
                    .title(Span::styled(" ⇥\u{a0}completes ", Theme::faint_style()))
                    .style(Style::default().bg(Theme::popover()));
                let inner = block.inner(rect);
                f.render_widget(block, rect);
                let buf = f.buffer_mut();
                for (i, user) in candidates.iter().enumerate() {
                    let picked = i == composer.mention_index.min(candidates.len() - 1);
                    let row = Rect {
                        x: inner.x,
                        y: inner.y + i as u16,
                        width: inner.width,
                        height: 1,
                    };
                    if picked {
                        buf.set_style(row, Style::default().bg(Theme::primary_soft()));
                    }
                    let line = Line::from(vec![
                        Span::raw(" "),
                        small_avatar(user),
                        Span::styled(
                            format!("@{}", user.username),
                            if picked {
                                Style::default().fg(Theme::on_primary_soft()).add_modifier(Modifier::BOLD)
                            } else {
                                Theme::text()
                            },
                        ),
                        Span::styled(
                            format!("  {}", truncate(user.display_name(), 18)),
                            if picked {
                                Style::default().fg(Theme::on_primary_soft())
                            } else {
                                Theme::dim()
                            },
                        ),
                    ]);
                    buf.set_line(row.x, row.y, &line, row.width);
                }
            }
        }
    }
}
