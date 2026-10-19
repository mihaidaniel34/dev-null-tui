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

/// A commit body as wrapped lines. Fenced code blocks keep their lines, on a washed background.
pub fn body_lines(body: &str, max: usize, base: Style) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code = false;

    for raw in body.trim_end().split('\n') {
        if raw.trim_start().starts_with("```") {
            in_code = !in_code;
            if in_code {
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
            let code = raw.replace('\t', "    ");
            let shown = truncate(&code, max.saturating_sub(2));
            let pad = max.saturating_sub(2 + width(&shown));
            lines.push(Line::from(vec![
                Span::styled("▏ ", Style::default().fg(Theme::rail()).bg(Theme::wash())),
                Span::styled(shown, Style::default().fg(Theme::ink()).bg(Theme::wash())),
                Span::styled(" ".repeat(pad), Style::default().bg(Theme::wash())),
            ]));
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
    fn links_show_without_their_scheme() {
        assert_eq!(
            plain(&body_lines("see https://dev-null.sh/terminal.", 40, Style::default())),
            ["see dev-null.sh/terminal."]
        );
    }
}
