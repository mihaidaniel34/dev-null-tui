//! Turning a commit body into wrapped, styled lines: fenced code blocks, `inline code`, @mentions and links, as
//! the web highlights them.

use chrono::{DateTime, Datelike, Local, Utc};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// Splits a line of prose into styled runs.
fn inline_spans(line: &str, base: Style) -> Vec<(String, Style)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(String, Style)> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;

    let flush = |plain: &mut String, out: &mut Vec<(String, Style)>| {
        if !plain.is_empty() {
            out.push((std::mem::take(plain), base));
        }
    };

    while i < chars.len() {
        let c = chars[i];
        let prev_word = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');

        if c == '`'
            && let Some(end) = chars[i + 1..].iter().position(|&x| x == '`')
        {
            flush(&mut plain, &mut out);
            let code: String = chars[i + 1..i + 1 + end].iter().collect();
            out.push((format!(" {code} "), Theme::code()));
            i += end + 2;
            continue;
        }

        let rest: String = chars[i..chars.len().min(i + 8)].iter().collect();
        if !prev_word && (rest.starts_with("https://") || rest.starts_with("http://")) {
            flush(&mut plain, &mut out);
            let mut end = i;
            while end < chars.len() && !chars[end].is_whitespace() {
                end += 1;
            }
            while end > i && matches!(chars[end - 1], '.' | ',' | ')' | '!' | '?' | ';' | ':' | '"' | '\'') {
                end -= 1;
            }
            let url: String = chars[i..end].iter().collect();
            let shown = url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .to_string();
            out.push((shown, Theme::link().add_modifier(Modifier::UNDERLINED)));
            i = end;
            continue;
        }

        if c == '@' && !prev_word && chars.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric()) {
            flush(&mut plain, &mut out);
            let mut end = i + 1;
            while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '-' || chars[end] == '_') {
                end += 1;
            }
            out.push((chars[i..end].iter().collect(), Theme::link()));
            i = end;
            continue;
        }

        plain.push(c);
        i += 1;
    }

    flush(&mut plain, &mut out);
    out
}

/// Greedy word wrap over styled runs. A word can span runs, like a mention followed by a slash.
pub fn wrap_spans(runs: Vec<(String, Style)>, max: usize) -> Vec<Vec<Span<'static>>> {
    let max = max.max(4);

    // Words are runs of non-space characters, whatever their style; inline code stays one word.
    let mut words: Vec<(Vec<(String, Style)>, bool)> = Vec::new();
    for (text, style) in runs {
        let atomic = style == Theme::code();
        for c in text.chars() {
            let space = c.is_whitespace() && !atomic;
            match words.last_mut() {
                Some((parts, was_space)) if *was_space == space => match parts.last_mut() {
                    Some((t, s)) if *s == style => t.push(c),
                    _ => parts.push((c.to_string(), style)),
                },
                _ => words.push((vec![(c.to_string(), style)], space)),
            }
        }
    }

    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut used = 0usize;

    for (parts, is_space) in words {
        let w: usize = parts.iter().map(|(t, _)| width(t)).sum();

        if used + w <= max {
            if !(is_space && used == 0 && lines.len() > 1) {
                lines.last_mut().unwrap().extend(parts.into_iter().map(|(t, s)| Span::styled(t, s)));
                used += w;
            }
            continue;
        }
        if is_space {
            lines.push(Vec::new());
            used = 0;
            continue;
        }
        if w <= max && used > 0 {
            lines.push(parts.into_iter().map(|(t, s)| Span::styled(t, s)).collect());
            used = w;
            continue;
        }

        // A word longer than the line breaks anywhere.
        for (text, style) in parts {
            let mut chunk = String::new();
            for c in text.chars() {
                let cw = c.width().unwrap_or(0);
                if used + cw > max {
                    if !chunk.is_empty() {
                        lines.last_mut().unwrap().push(Span::styled(std::mem::take(&mut chunk), style));
                    }
                    lines.push(Vec::new());
                    used = 0;
                }
                chunk.push(c);
                used += cw;
            }
            if !chunk.is_empty() {
                lines.last_mut().unwrap().push(Span::styled(chunk, style));
            }
        }
    }

    for line in lines.iter_mut() {
        while line
            .last()
            .is_some_and(|s| s.style != Theme::code() && s.content.chars().all(char::is_whitespace))
        {
            line.pop();
        }
    }
    lines
}

pub fn wrap_plain(text: &str, max: usize, style: Style) -> Vec<Line<'static>> {
    text.split('\n')
        .flat_map(|l| wrap_spans(vec![(l.to_string(), style)], max))
        .map(Line::from)
        .collect()
}

/// Wraps text with every line indented, for hanging descriptions.
pub fn wrap_indented(text: &str, max: usize, style: Style, indent: usize) -> Vec<Line<'static>> {
    wrap_plain(text, max.saturating_sub(indent), style)
        .into_iter()
        .map(|line| Line::from([vec![Span::raw(" ".repeat(indent))], line.spans].concat()))
        .collect()
}

/// The colors the web gives tempest/highlight's classes, from the app's own palette.
fn token_style(class: &str) -> Option<Style> {
    Some(match class {
        "hl-keyword" => Style::default().fg(Theme::primary()),
        "hl-type" | "hl-generic" => Style::default().fg(Theme::brand()),
        "hl-property" => Style::default().fg(Theme::lane(3)),
        "hl-value" | "hl-addition" => Style::default().fg(Theme::lane(1)),
        "hl-number" | "hl-literal" => Style::default().fg(Theme::lane(0)),
        "hl-variable" => Style::default().fg(Theme::lane(4)),
        "hl-comment" => Style::default().fg(Theme::muted()).add_modifier(Modifier::ITALIC),
        "hl-attribute" => Style::default().add_modifier(Modifier::ITALIC),
        "hl-deletion" => Style::default().fg(Theme::destructive()),
        _ => return None,
    })
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A highlighted code block's HTML as lines of styled runs. Nested spans keep the innermost class that has a color.
pub fn highlighted_lines(html: &str) -> Vec<Vec<(String, Style)>> {
    let base = Style::default().fg(Theme::ink());
    let mut lines: Vec<Vec<(String, Style)>> = vec![Vec::new()];
    let mut stack: Vec<Style> = vec![base];
    let mut rest = html;
    while !rest.is_empty() {
        if rest.starts_with("</span>") {
            if stack.len() > 1 {
                stack.pop();
            }
            rest = &rest[7..];
        } else if rest.starts_with("<span") {
            let end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
            let tag = &rest[..end];
            let current = *stack.last().unwrap_or(&base);
            let class = tag.split("class=\"").nth(1).and_then(|c| c.split('"').next()).unwrap_or("");
            let style = class.split_whitespace().filter_map(token_style).fold(current, |s, t| s.patch(t));
            stack.push(style);
            rest = &rest[end..];
        } else if rest.starts_with('<') {
            // Any other tag carries no color.
            let end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
            rest = &rest[end..];
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let style = *stack.last().unwrap_or(&base);
            for (i, part) in unescape(&rest[..end]).split('\n').enumerate() {
                if i > 0 {
                    lines.push(Vec::new());
                }
                if !part.is_empty() {
                    lines.last_mut().unwrap().push((part.replace('\t', "    "), style));
                }
            }
            rest = &rest[end..];
        }
    }
    lines
}

/// A commit body as wrapped lines. Fenced code blocks keep their lines, on a washed background.
pub fn body_lines(body: &str, max: usize, base: Style) -> Vec<Line<'static>> {
    body_lines_highlighted(body, max, base, &[])
}

/// Same, with each code block's highlighting from the server, in order, where there is one.
pub fn body_lines_highlighted(body: &str, max: usize, base: Style, blocks: &[String]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code = false;
    let mut block = 0usize;
    let mut block_line = 0usize;
    let mut highlighted: Vec<Vec<(String, Style)>> = Vec::new();

    for raw in body.trim_end().split('\n') {
        if raw.trim_start().starts_with("```") {
            in_code = !in_code;
            if in_code {
                highlighted = blocks.get(block).map(|html| highlighted_lines(html)).unwrap_or_default();
                block += 1;
                block_line = 0;
                let lang = raw.trim().trim_start_matches('`').trim();
                if !lang.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!(" {lang} "),
                        Theme::faint_style().bg(Theme::wash()),
                    )));
                }
            }
            continue;
        }

        if in_code {
            let room = max.saturating_sub(2);
            let mut spans = vec![Span::styled("▏ ", Style::default().fg(Theme::rail()).bg(Theme::wash()))];
            let mut used = 0;
            match highlighted.get(block_line) {
                Some(runs) => {
                    for (text, style) in runs {
                        if used >= room {
                            break;
                        }
                        let shown = truncate(text, room - used);
                        used += width(&shown);
                        spans.push(Span::styled(shown, style.bg(Theme::wash())));
                    }
                }
                None => {
                    let shown = truncate(&raw.replace('\t', "    "), room);
                    used = width(&shown);
                    spans.push(Span::styled(shown, Style::default().fg(Theme::ink()).bg(Theme::wash())));
                }
            }
            spans.push(Span::styled(
                " ".repeat(room.saturating_sub(used)),
                Style::default().bg(Theme::wash()),
            ));
            lines.push(Line::from(spans));
            block_line += 1;
            continue;
        }

        if raw.trim().is_empty() {
            lines.push(Line::default());
            continue;
        }

        for spans in wrap_spans(inline_spans(raw, base), max) {
            lines.push(Line::from(spans));
        }
    }

    // A fence never closed still shows its code; a trailing blank line never does.
    while lines.last().is_some_and(|l| l.width() == 0) {
        lines.pop();
    }
    lines
}

pub fn relative_time(at: Option<DateTime<Utc>>) -> String {
    let Some(at) = at else { return String::new() };
    let seconds = (Utc::now() - at).num_seconds().max(0);
    match seconds {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        86_400..=604_799 => format!("{}d", seconds / 86_400),
        _ => {
            let local = at.with_timezone(&Local);
            if local.year() == Local::now().year() {
                local.format("%-d %b").to_string()
            } else {
                local.format("%-d %b %Y").to_string()
            }
        }
    }
}

/// "Today", "Yesterday", or the date, as the log's day separators say.
pub fn day_label(at: DateTime<Utc>) -> String {
    let day = at.with_timezone(&Local).date_naive();
    let today = Local::now().date_naive();
    match (today - day).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => day.format("%A").to_string(),
        _ if day.year() == today.year() => day.format("%a %-d %b").to_string(),
        _ => day.format("%a %-d %b %Y").to_string(),
    }
}

pub fn domain(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
        .unwrap_or_default()
}

pub fn count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1000..=9_999 => format!("{:.1}k", n as f64 / 1000.0).replace(".0k", "k"),
        _ => format!("{}k", n / 1000),
    }
}

/// A path with the home directory as ~, the way it's written in a shell.
pub fn tilde(path: &std::path::Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(|rest| rest.display().to_string())) {
        Some(rest) => format!("~/{rest}"),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    }

    #[test]
    fn it_wraps_on_words() {
        assert_eq!(
            plain(&body_lines("one two three four", 9, Style::default())),
            ["one two", "three", "four"]
        );
    }

    #[test]
    fn a_mention_and_what_follows_it_wrap_together() {
        assert_eq!(
            plain(&body_lines("I built @madinco/smith today", 16, Style::default())),
            ["I built", "@madinco/smith", "today"]
        );
    }

    #[test]
    fn code_blocks_keep_their_lines() {
        let lines = plain(&body_lines("look:\n```bash\nls -la\n```", 20, Style::default()));
        assert_eq!(lines[0], "look:");
        assert!(lines.iter().any(|l| l.contains("ls -la")));
    }

    #[test]
    fn it_reads_highlighted_html() {
        let lines = highlighted_lines(
            "<span class=\"hl-keyword\">fn</span> main() {\n    <span class=\"hl-comment\">// a &lt;b&gt; &amp; c</span>\n}",
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0][0].0, "fn");
        assert_eq!(lines[0][0].1.fg, Some(Theme::primary()));
        assert_eq!(lines[1].iter().map(|(t, _)| t.as_str()).collect::<String>(), "    // a <b> & c");
        let body = "look\n```rust\nfn main() {\n}\n```";
        let rendered = plain(&body_lines_highlighted(
            body,
            30,
            Style::default(),
            &["<span class=\"hl-keyword\">fn</span> main() {\n}".into()],
        ));
        assert!(rendered.iter().any(|l| l.contains("fn main() {")));
    }

    #[test]
    fn links_show_without_their_scheme() {
        assert_eq!(
            plain(&body_lines("see https://dev-null.sh/terminal.", 40, Style::default())),
            ["see dev-null.sh/terminal."]
        );
    }
}
