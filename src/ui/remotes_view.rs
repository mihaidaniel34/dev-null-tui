//! Remotes on screen: the inbox, a conversation, and the picker for starting one. Its state and keys are in
//! remotes.rs; the web's Remotes page (the screenshots in the README's history) is what it follows.

use chrono::Local;

use super::*;
use crate::remotes::{Chat, InboxTab, REACTIONS, access_notice};
use crate::text::body_lines_highlighted;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    match app.screen().clone() {
        Screen::Remotes => draw_inbox(f, app, area),
        Screen::Remote(handle) => draw_chat(f, app, area, &handle),
        _ => {}
    }
}

/// A note in the middle of what would be a list.
fn note(buf: &mut Buffer, area: Rect, text: &str, style: Style) {
    let lines = wrap_plain(text, (area.width as usize).saturating_sub(6).max(10), style);
    let top = area.y + area.height.saturating_sub(lines.len() as u16) / 3;
    for (i, line) in lines.iter().enumerate() {
        let x = area.x + area.width.saturating_sub(line.width() as u16) / 2;
        if top + (i as u16) < area.y + area.height {
            buf.set_line(x, top + i as u16, line, area.width);
        }
    }
}

fn first_line(text: &str) -> String {
    text.split('\n')
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// What a message says in one line: its text, or what it carries instead.
fn gist(message: &Message) -> String {
    let text = first_line(message.body.as_deref().unwrap_or_default());
    if !text.is_empty() {
        return text;
    }
    if message.voice.is_some() {
        return "♪ voice note".into();
    }
    match message.images.as_ref().map_or(0, Vec::len) {
        0 => String::new(),
        1 => "▣ image".into(),
        n => format!("▣ {n} images"),
    }
}

// Inbox --------------------------------------------------------------------------------------------------------

fn draw_inbox(f: &mut Frame, app: &mut App, area: Rect) {
    let mut y = draw_header(
        f.buffer_mut(),
        app,
        Rect {
            y: area.y,
            height: 1,
            ..area
        },
        "Remotes",
    );
    let r = &app.remotes;

    draw_tabs(
        f.buffer_mut(),
        Rect { y, height: 2, ..area },
        &InboxTab::ALL.map(|tab| (tab.title(), tab == r.tab)),
    );
    y += 2;
    hairline(f.buffer_mut(), area.x, y, area.width);
    y += 1;

    let searching = !r.query.is_empty() || r.searching;
    let search = if searching {
        Line::from(vec![
            Span::styled("   ◎  ", Theme::dim()),
            Span::styled(r.query.clone(), Theme::text()),
        ])
    } else {
        Line::from(vec![Span::styled("   ◎  Search names or @handles", Theme::faint_style())])
    };
    f.buffer_mut().set_line(area.x, y, &search, area.width);
    if !searching {
        f.buffer_mut().set_string(
            area.x + area.width - 4,
            y,
            " / ",
            Style::default().fg(Theme::muted()).bg(Theme::wash()),
        );
    }
    if r.searching {
        f.set_cursor_position((area.x + 6 + width(&r.query) as u16, y));
    }
    y += 1;
    hairline(f.buffer_mut(), area.x, y, area.width);
    y += 1;

    if r.requests > 0 {
        let text = format!(
            "   {} request{} waiting. Merge {} on the web.",
            r.requests,
            if r.requests == 1 { "" } else { "s" },
            if r.requests == 1 { "it" } else { "them" }
        );
        f.buffer_mut().set_string(area.x, y, text, Style::default().fg(Theme::lane(2)));
        y += 1;
    }

    let body = Rect {
        y,
        height: area.height.saturating_sub(y - area.y),
        ..area
    };
    if let Some(notice) = &r.notice {
        return note(f.buffer_mut(), body, notice, Theme::dim());
    }
    if r.list.is_empty() {
        let spinner = SPINNER[app.tick % SPINNER.len()];
        let text = match (&r.error, r.loaded) {
            (Some(error), _) => format!("{error}\n\nR to try again."),
            (None, false) => format!("{spinner}  Loading"),
            (None, true) if !r.query.trim().is_empty() => "Nobody matches that.".into(),
            (None, true) if r.tab == InboxTab::Stashed => "Nothing stashed. x stashes a conversation.".into(),
            (None, true) => "No remotes yet. n starts one with someone who follows you.".into(),
        };
        let style = if r.error.is_some() {
            Style::default().fg(Theme::destructive())
        } else {
            Theme::dim()
        };
        return note(f.buffer_mut(), body, &text, style);
    }

    const ROW: u16 = 3;
    let visible = (body.height / ROW).max(1) as usize;
    let scroll = if r.selected < r.scroll {
        r.selected
    } else if r.selected >= r.scroll + visible {
        r.selected + 1 - visible
    } else {
        r.scroll
    };

    for (n, conversation) in r.list.iter().enumerate().skip(scroll).take(visible) {
        let top = body.y + (n - scroll) as u16 * ROW;
        let selected = n == r.selected;
        if selected {
            f.buffer_mut()
                .set_style(Rect { y: top, height: 2, ..body }, Style::default().bg(Theme::wash()));
        }
        let [first, second] = inbox_row(conversation, body.width as usize, selected);
        f.buffer_mut().set_line(body.x, top, &first, body.width);
        f.buffer_mut().set_line(body.x, top + 1, &second, body.width);
    }
    app.remotes.scroll = scroll;
}

/// A conversation's two lines: who and when, then the last message and whether there is more to pull.
fn inbox_row(conversation: &Conversation, w: usize, selected: bool) -> [Line<'static>; 2] {
    let partner = conversation.partner.as_ref();
    let lane = partner.map_or(Theme::rail(), Theme::user_lane);
    let avatar = |row: u16| -> Span<'static> {
        partner.and_then(|p| avatar_row(p, 4, 2, row)).unwrap_or_else(|| {
            if row == 0 {
                Span::styled(" ●  ", Style::default().fg(lane))
            } else {
                Span::raw("    ")
            }
        })
    };

    let name = match partner {
        Some(p) => p.display_name().to_string(),
        None => conversation.name.clone().unwrap_or_else(|| "Org".into()),
    };
    let handle = partner.map(|p| format!(" @{}", p.username)).unwrap_or_else(|| " org".into());
    let age = relative_time(conversation.last_message_at);
    let text_w = w.saturating_sub(7 + age.len() + 2);

    let name = truncate(&name, text_w);
    let handle = truncate(&handle, text_w.saturating_sub(width(&name)));
    let pad = w.saturating_sub(6 + width(&name) + width(&handle) + age.len() + 2);
    let mut first = vec![bar(selected), Span::raw(" "), avatar(0), Span::raw(" ")];
    first.push(Span::styled(name, Theme::bold()));
    first.push(Span::styled(handle, Theme::faint_style()));
    first.push(Span::raw(" ".repeat(pad)));
    first.push(Span::styled(age, Theme::faint_style()));

    let unread = conversation.ahead;
    let preview = conversation.last_message.as_ref().map_or(String::new(), |m| {
        let mine = partner.is_some_and(|p| m.user_id != p.id);
        format!("{}{}", if mine { "you: " } else { "" }, gist(m))
    });
    let badge = if unread > 0 { format!(" {unread} ") } else { String::new() };
    let preview = truncate(&preview, w.saturating_sub(7 + badge.len() + 2));
    let pad = w.saturating_sub(6 + width(&preview) + badge.len() + 2);
    let mut second = vec![bar(selected), Span::raw(" "), avatar(1), Span::raw(" ")];
    second.push(Span::styled(preview, if unread > 0 { Theme::bold() } else { Theme::dim() }));
    second.push(Span::raw(" ".repeat(pad)));
    if unread > 0 {
        second.push(Span::styled(
            badge,
            Style::default()
                .fg(Theme::on_primary())
                .bg(Theme::primary())
                .add_modifier(Modifier::BOLD),
        ));
    }
    [Line::from(first), Line::from(second)]
}

// A conversation -----------------------------------------------------------------------------------------------

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect, handle: &str) {
    let Some(chat) = app.remotes.chats.get(handle) else { return };
    let partner = chat.partner.clone();
    let me = app.me.as_ref().map(|me| me.user.id.clone()).unwrap_or_default();
    let spinner = SPINNER[app.tick % SPINNER.len()];
    let back = app.stack.len() > 1;

    // The header: who it is, and what they're doing.
    let name = partner.as_ref().map_or(handle.to_string(), |p| p.display_name().to_string());
    let mut header = vec![Span::styled(if back { " ← " } else { "   " }, Theme::dim())];
    if let Some(p) = &partner {
        header.push(small_avatar(p));
    }
    header.push(Span::styled(name.clone(), Theme::bold()));
    header.push(Span::styled(format!("  @{handle}"), Theme::faint_style()));
    if chat.is_archived {
        header.push(Span::styled("  stashed", Theme::faint_style()));
    }
    if chat.partner_typing {
        header.push(Span::styled("  typing…", Theme::link()));
    }
    {
        let buf = f.buffer_mut();
        buf.set_line(area.x, area.y + 1, &Line::from(header), area.width);
        if back {
            buf.set_string(area.x + area.width - 4, area.y + 1, "esc", Theme::faint_style());
        }
        hairline(buf, area.x, area.y + 2, area.width);
    }

    // The composer sits at the bottom: an optional line about what is replied to, a rule, the field and a hint.
    let reply = chat.reply_to.and_then(|id| chat.messages.iter().find(|m| m.id == id)).map(|m| {
        let who = if partner.as_ref().is_some_and(|p| m.user_id == p.id) {
            name.clone()
        } else {
            "yourself".into()
        };
        format!("↪ Replying to {who}: {}", gist(m))
    });
    let composer_h: u16 = 3 + u16::from(reply.is_some());
    let thread = Rect {
        y: area.y + 3,
        height: area.height.saturating_sub(3 + composer_h),
        ..area
    };
    let composer = Rect {
        y: area.y + area.height - composer_h,
        height: composer_h,
        ..area
    };

    // The messages.
    let mut top = chat.top;
    if let Some(partner) = partner.as_ref().filter(|_| !chat.messages.is_empty()) {
        let (lines, ranges) = thread_lines(chat, partner, &me, area.width as usize);
        let height = thread.height as usize;
        let last = chat.selected + 1 >= chat.messages.len();
        let (start, end) = ranges.get(chat.selected).copied().unwrap_or((0, lines.len()));
        top = top.min(lines.len().saturating_sub(height));
        if last {
            top = lines.len().saturating_sub(height);
        } else if start < top || end - start >= height {
            top = start;
        } else if end > top + height {
            top = end - height;
        }
        let buf = f.buffer_mut();
        for (i, line) in lines.iter().skip(top).take(height).enumerate() {
            buf.set_line(area.x, thread.y + i as u16, line, area.width);
        }
        if chat.older_loading {
            buf.set_string(area.x + 3, thread.y, format!("{spinner} older messages"), Theme::faint_style());
        }
    } else if let Some(error) = &chat.error {
        note(
            f.buffer_mut(),
            thread,
            &format!("{error}\n\nR to try again."),
            Style::default().fg(Theme::destructive()),
        );
    } else if !chat.loaded {
        note(f.buffer_mut(), thread, &format!("{spinner}  Loading"), Theme::dim());
    } else if let Some(reason) = access_notice(&chat.access, handle) {
        note(f.buffer_mut(), thread, &reason, Theme::dim());
    } else {
        note(
            f.buffer_mut(),
            thread,
            &format!("Nothing here yet. i writes the first message to {name}."),
            Theme::dim(),
        );
    }

    draw_composer(f, chat, composer, handle, reply);
    if let Some(chat) = app.remotes.chats.get_mut(handle) {
        chat.top = top;
    }
}

fn draw_composer(f: &mut Frame, chat: &Chat, area: Rect, handle: &str, reply: Option<String>) {
    let mut y = area.y;
    if let Some(reply) = reply {
        f.buffer_mut()
            .set_string(area.x + 2, y, truncate(&reply, area.width as usize - 4), Theme::dim());
        y += 1;
    }
    hairline(f.buffer_mut(), area.x, y, area.width);
    y += 1;

    let open = chat.access == "open" || !chat.loaded;
    let push = if chat.sending {
        " Pushing… ".to_string()
    } else {
        " Push ⏎ ".to_string()
    };
    let field_w = (area.width as usize).saturating_sub(5 + push.len());
    let text: String = chat.input.iter().map(|c| if *c == '\n' { '↵' } else { *c }).collect();
    let before: String = text.chars().take(chat.cursor).collect();

    let (shown, cursor_x) = if width(&before) < field_w {
        (text.clone(), width(&before))
    } else {
        let skip = width(&before) - field_w + 1;
        (text.chars().skip(skip).collect::<String>(), field_w - 1)
    };
    let field = if !open {
        Span::styled(
            truncate(&access_notice(&chat.access, handle).unwrap_or_default(), field_w),
            Theme::faint_style(),
        )
    } else if text.is_empty() {
        Span::styled(format!("Message @{handle}"), Theme::faint_style())
    } else {
        Span::styled(truncate(&shown, field_w), Theme::text())
    };
    let prompt = if chat.typing { Theme::link() } else { Theme::faint_style() };
    let buf = f.buffer_mut();
    buf.set_line(
        area.x + 2,
        y,
        &Line::from(vec![Span::styled("› ", prompt), field]),
        area.width.saturating_sub(2),
    );
    if open {
        let style = if chat.typing && !chat.input.is_empty() {
            Style::default()
                .fg(Theme::on_primary())
                .bg(Theme::primary())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::muted()).bg(Theme::wash())
        };
        buf.set_string(area.x + area.width - push.len() as u16 - 1, y, push, style);
    }
    if chat.typing {
        f.set_cursor_position((area.x + 4 + cursor_x as u16, y));
    }
    y += 1;

    let hint: Line<'static> = if chat.react {
        let mut spans = vec![Span::styled("react  ", Theme::dim())];
        for (i, emoji) in REACTIONS.iter().enumerate() {
            spans.push(key_chip(&(i + 1).to_string()));
            spans.push(Span::styled(format!(" {emoji}  "), Theme::text()));
        }
        spans.push(Span::styled("any other key cancels", Theme::faint_style()));
        Line::from(spans)
    } else if chat.typing {
        Line::from(vec![
            key_chip("⏎"),
            Span::styled(" push   ", Theme::faint_style()),
            key_chip("alt ⏎"),
            Span::styled(" new line   ", Theme::faint_style()),
            key_chip("esc"),
            Span::styled(" stop writing", Theme::faint_style()),
        ])
    } else {
        Line::from(vec![
            key_chip("i"),
            Span::styled(" write   ", Theme::faint_style()),
            key_chip("r"),
            Span::styled(" reply   ", Theme::faint_style()),
            key_chip("e"),
            Span::styled(" react   ", Theme::faint_style()),
            key_chip("x"),
            Span::styled(if chat.is_archived { " unstash" } else { " stash" }, Theme::faint_style()),
        ])
    };
    f.buffer_mut().set_line(area.x + 2, y, &hint, area.width.saturating_sub(2));
}

/// Every message as lines, and where each one starts and ends. Their own are on the right, in the accent color,
/// and the partner's on the left, with the time under the last of each run and when the partner pulled, if they share.
fn thread_lines(chat: &Chat, partner: &User, me: &str, w: usize) -> (Vec<Line<'static>>, Vec<(usize, usize)>) {
    let content_max = (w.saturating_sub(8) * 3 / 4).clamp(12, 72);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut ranges = Vec::with_capacity(chat.messages.len());

    // Their last message the partner has pulled, when they share pulls.
    let pulled = chat
        .messages
        .iter()
        .filter(|m| m.user_id != partner.id && m.id <= chat.partner_read_through)
        .map(|m| m.id)
        .next_back()
        .filter(|_| chat.partner_read_at.is_some());

    let mut previous: Option<&Message> = None;
    for (i, message) in chat.messages.iter().enumerate() {
        let mine = message.user_id != partner.id;
        let selected = i == chat.selected;

        if let Some(at) = message.created_at {
            let new_day = previous
                .and_then(|p| p.created_at)
                .is_none_or(|p| p.with_timezone(&Local).date_naive() != at.with_timezone(&Local).date_naive());
            if new_day {
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    format!("{:^w$}", day_label(at), w = w),
                    Theme::faint_style(),
                )));
            }
        }
        let same_run = previous.is_some_and(|p| (p.user_id == message.user_id) && !is_gap(p, message));
        if !same_run && previous.is_some() {
            lines.push(Line::default());
        }

        let start = lines.len();
        if message.event.is_some() {
            lines.push(Line::from(Span::styled(
                format!("{:^w$}", gist(message), w = w),
                Theme::faint_style(),
            )));
            ranges.push((start, lines.len()));
            previous = Some(message);
            continue;
        }

        let block = bubble(message, partner, me, mine, content_max);
        let block_w = block.iter().map(|l| l.width()).max().unwrap_or(0);
        let place = |mut spans: Vec<Span<'static>>, own_w: usize| -> Line<'static> {
            let mut line = vec![bar(selected)];
            if mine {
                line.push(Span::raw(" ".repeat(w.saturating_sub(block_w.max(own_w) + 2))));
            } else {
                line.push(Span::raw(" "));
            }
            line.append(&mut spans);
            Line::from(line)
        };
        for line in block {
            let own = line.width();
            lines.push(place(line.spans, own));
        }

        // Reactions, as chips under the bubble.
        let mut chips: Vec<(String, usize, bool)> = Vec::new();
        for reaction in &message.reactions {
            match chips.iter_mut().find(|c| c.0 == reaction.emoji) {
                Some(chip) => {
                    chip.1 += 1;
                    chip.2 |= reaction.user_id == me;
                }
                None => chips.push((reaction.emoji.clone(), 1, reaction.user_id == me)),
            }
        }
        if !chips.is_empty() {
            let mut spans = Vec::new();
            for (emoji, n, own) in chips {
                let style = if own {
                    Style::default().fg(Theme::on_primary_soft()).bg(Theme::primary_soft())
                } else {
                    Style::default().fg(Theme::ink()).bg(Theme::wash())
                };
                spans.push(Span::styled(format!(" {emoji} {n} "), style));
                spans.push(Span::raw(" "));
            }
            let own = spans.iter().map(|s| s.width()).sum();
            lines.push(place(spans, own));
        }

        // The time, under the last message of a run, and whether it has been pulled.
        let next = chat.messages.get(i + 1);
        let run_ends = next.is_none_or(|n| n.user_id != message.user_id || is_gap(message, n));
        let pulled_here = pulled == Some(message.id);
        if run_ends || pulled_here {
            let mut parts = Vec::new();
            if run_ends && let Some(at) = message.created_at {
                parts.push(Span::styled(
                    at.with_timezone(&Local).format("%-I:%M %p").to_string(),
                    Theme::faint_style(),
                ));
            }
            if pulled_here && let Some(at) = chat.partner_read_at {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                parts.push(Span::styled("● ", Style::default().fg(Theme::user_lane(partner))));
                parts.push(Span::styled(
                    format!("pulled {}", at.with_timezone(&Local).format("%-I:%M %p")),
                    Theme::faint_style(),
                ));
            }
            let own = parts.iter().map(|s| s.width()).sum();
            lines.push(place(parts, own));
        }

        ranges.push((start, lines.len()));
        previous = Some(message);
    }

    if chat.partner_typing {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!("   {} is typing…", partner.display_name()),
            Theme::link(),
        )));
    }
    (lines, ranges)
}

/// Whether enough time has passed between two messages to start a new run, with a time of its own.
fn is_gap(a: &Message, b: &Message) -> bool {
    match (a.created_at, b.created_at) {
        (Some(a), Some(b)) => (b - a).num_minutes() >= 5,
        _ => false,
    }
}

/// One message's bubble: what it replies to, its text, pictures, and a link card, padded to a common width and
/// in the colors of whoever wrote it.
fn bubble(message: &Message, partner: &User, me: &str, mine: bool, max: usize) -> Vec<Line<'static>> {
    let style = if mine {
        Style::default().fg(Theme::on_primary()).bg(Theme::primary())
    } else {
        Style::default().fg(Theme::ink()).bg(Theme::wash())
    };
    let dim = if mine { style } else { Theme::dim().bg(Theme::wash()) };

    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut pictures: Vec<Line<'static>> = Vec::new();

    if let Some(reply) = &message.reply_to {
        let who = if reply.user_id == partner.id {
            partner.display_name().to_string()
        } else if reply.user_id == me {
            "You".into()
        } else {
            String::new()
        };
        let text = first_line(reply.body.as_deref().unwrap_or_default());
        let text = if !text.is_empty() {
            text
        } else if reply.has_voice {
            "♪ voice note".into()
        } else if reply.images_count > 0 {
            "▣ image".into()
        } else {
            String::new()
        };
        rows.push(Line::from(Span::styled(
            format!("┃ {}", truncate(&who, max)),
            style.add_modifier(Modifier::BOLD),
        )));
        rows.push(Line::from(Span::styled(
            format!("┃ {}", truncate(&text, max.saturating_sub(2))),
            dim,
        )));
    }

    let body = message.body.as_deref().unwrap_or_default();
    if !body.trim().is_empty() {
        let blocks = message.highlighted_blocks();
        rows.extend(body_lines_highlighted(body, max, style, &blocks));
    }

    if let Some(images) = message.images.as_ref().filter(|i| !i.is_empty()) {
        match picture_lines(images, max.min(40), 8) {
            Some(lines) => pictures = lines,
            None => {
                for image in images {
                    let name = image.url.as_ref().map(|_| "▣ image").unwrap_or("▣ image");
                    rows.push(Line::from(Span::styled(name.to_string(), style)));
                }
                rows.push(Line::from(Span::styled("v views images", dim)));
            }
        }
    }
    if message.voice.is_some() {
        rows.push(Line::from(Span::styled("♪ voice note (play it in the app)", style)));
    }
    if let Some(card) = message
        .link_preview
        .as_ref()
        .and_then(|p| p.title.clone().filter(|t| !t.is_empty()))
    {
        rows.push(Line::from(Span::styled(
            format!("↗ {}", truncate(&card, max.saturating_sub(2))),
            dim,
        )));
    }
    if rows.is_empty() && pictures.is_empty() {
        rows.push(Line::from(Span::styled("…", dim)));
    }

    let inner = rows
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .max(pictures.first().map_or(0, |l| l.width()));
    let mut out: Vec<Line<'static>> = Vec::new();
    for row in rows {
        let used = row.width();
        let mut spans = vec![Span::styled(" ", style)];
        for mut span in row.spans {
            span.style = if mine {
                span.style.fg(Theme::on_primary()).bg(Theme::primary())
            } else {
                span.style.bg(Theme::wash())
            };
            spans.push(span);
        }
        spans.push(Span::styled(" ".repeat(inner - used + 1), style));
        out.push(Line::from(spans));
    }
    for row in pictures {
        let mut spans = vec![Span::raw(" ")];
        spans.extend(row.spans);
        out.push(Line::from(spans));
    }
    out
}

// Starting one -------------------------------------------------------------------------------------------------

pub fn draw_picker(f: &mut Frame, app: &App, area: Rect) {
    let Some(picker) = &app.remotes.picker else { return };
    let rect = centered(area, 64, 20);
    let inner = modal(f, rect, "New remote");
    let w = inner.width as usize;

    let mut lines = vec![
        Line::from(Span::styled(
            "Push to someone. They need Remotes on, and usually to follow you.",
            Theme::dim(),
        )),
        Line::default(),
    ];
    let field_y = inner.y + lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled("› ", Theme::link()),
        if picker.query.is_empty() {
            Span::styled("Search names or @handles", Theme::faint_style())
        } else {
            Span::styled(picker.query.clone(), Theme::text())
        },
    ]));
    lines.push(Line::default());

    let room = (inner.height as usize).saturating_sub(lines.len() + 2);
    let start = picker.selected.saturating_sub(room.saturating_sub(1));
    for (n, candidate) in picker.candidates.iter().enumerate().skip(start).take(room) {
        let selected = n == picker.selected;
        let blocked = candidate.reason.as_deref().is_some_and(|r| !r.is_empty());
        let user = &candidate.user;
        let name = truncate(user.display_name(), w / 2);
        let mut spans = vec![bar(selected), Span::raw(" "), small_avatar(user)];
        spans.push(Span::styled(name.clone(), if blocked { Theme::dim() } else { Theme::bold() }));
        spans.push(Span::styled(format!(" @{}", user.username), Theme::faint_style()));
        if let Some(reason) = candidate.reason.as_deref().filter(|r| !r.is_empty()) {
            let used = 5 + width(&name) + user.username.len() + 2;
            spans.push(Span::styled(
                format!("  {}", truncate(reason, w.saturating_sub(used + 2))),
                Style::default().fg(Theme::lane(2)),
            ));
        }
        lines.push(Line::from(spans));
    }
    if picker.candidates.is_empty() {
        let text = match (&picker.error, picker.loading) {
            (Some(error), _) => error.clone(),
            (None, true) => "Searching…".into(),
            (None, false) => "Nobody matches that.".into(),
        };
        lines.push(Line::from(Span::styled(text, Theme::dim())));
    }

    f.render_widget(Paragraph::new(lines), inner);
    let hint_y = inner.y + inner.height - 1;
    f.buffer_mut().set_line(
        inner.x,
        hint_y,
        &Line::from(vec![
            key_chip("⏎"),
            Span::styled(" open   ", Theme::faint_style()),
            key_chip("↑↓"),
            Span::styled(" pick   ", Theme::faint_style()),
            key_chip("esc"),
            Span::styled(" cancel", Theme::faint_style()),
        ]),
        inner.width,
    );
    f.set_cursor_position((inner.x + 2 + width(&picker.query) as u16, field_y));
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::api::Api;
    use crate::config::Config;
    use crate::remotes::Chat;

    fn user(id: &str, handle: &str, name: &str) -> User {
        User {
            id: id.into(),
            username: handle.into(),
            name: Some(name.into()),
            ..Default::default()
        }
    }

    fn message(id: i64, from: &str, body: &str, minute: u32) -> Message {
        Message {
            id,
            user_id: from.into(),
            body: Some(body.into()),
            created_at: chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 10, 27, 17, minute, 0).single(),
            ..Default::default()
        }
    }

    fn app() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Api::new("http://127.0.0.1:9".into(), None), Config::default(), tx);
        app.me = Some(CurrentUser {
            user: user("1", "mihai", "Mihai"),
            ..Default::default()
        });
        app
    }

    fn screenshot(app: &mut App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| super::super::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn a_conversation_draws_both_sides() {
        let mut app = app();
        let mut chat = Chat {
            partner: Some(user("2", "petox03", "Alberto Sosa")),
            loaded: true,
            access: "open".into(),
            ..Default::default()
        };
        chat.messages = vec![
            message(1, "2", "Hey bro, im trying out your TUI, but it won't let me log in", 47),
            message(2, "2", "Am I doing something wrong?", 47),
            message(3, "1", "uhh are you on the latest version there was a login issue in 0.1", 52),
            message(4, "1", "yeah 0.4.3 is out try that", 53),
        ];
        chat.messages[3].reactions = vec![Reaction {
            user_id: "2".into(),
            emoji: REACTIONS[0].into(),
        }];
        chat.messages[2].reply_to = Some(ReplyTo {
            user_id: "2".into(),
            body: Some("Am I doing something wrong?".into()),
            ..Default::default()
        });
        chat.partner_read_through = 4;
        chat.partner_read_at = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 10, 27, 18, 7, 0).single();
        chat.selected = 3;
        app.remotes.chats.insert("petox03".into(), chat);
        app.stack = vec![Screen::Remotes, Screen::Remote("petox03".into())];

        let shown = screenshot(&mut app, 110, 42);
        println!("{shown}");
        assert!(shown.contains("Alberto Sosa"));
        assert!(shown.contains("won't let me"));
        assert!(shown.contains("┃ Alberto Sosa"));
        assert!(shown.contains("pulled"));
        assert!(shown.contains("Message @petox03"));
        // Their messages hug the left edge of the thread; ours the right.
        let theirs = shown.lines().find(|l| l.contains("won't let me")).unwrap();
        let ours = shown.lines().find(|l| l.contains("is out try that")).unwrap();
        assert!(ours.find("yeah").unwrap() > theirs.find("Hey").unwrap() + 20);
    }

    #[tokio::test]
    async fn the_inbox_lists_conversations() {
        let mut app = app();
        app.remotes.loaded = true;
        app.remotes.list = vec![Conversation {
            id: "a".into(),
            partner: Some(user("2", "petox03", "Alberto Sosa")),
            last_message: Some(message(9, "1", "ok i tested it and it's fixed", 10)),
            last_message_at: Some(chrono::Utc::now()),
            ahead: 3,
            ..Default::default()
        }];
        app.stack = vec![Screen::Remotes];
        let shown = screenshot(&mut app, 110, 24);
        println!("{shown}");
        assert!(shown.contains("Alberto Sosa @petox03"));
        assert!(shown.contains("you: ok i tested it"));
        assert!(shown.contains(" 3 "));
        assert!(shown.contains("Stashed"));
    }
}
