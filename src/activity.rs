//! A profile's activity, drawn as the web draws it: a contribution graph for the year, or bars for a newer account's
//! day, week or month, in the user's lane color. Plus their streaks, Unix rank and badges.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::model::{Achievements, Activity, Bucket, Streaks};
use crate::text::count;
use crate::theme::{Theme, rgb};

const SHELL: u32 = 0x111112;
const EMPTY: u32 = 0x232326;
const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

fn blend(color: u32, t: f32) -> u32 {
    let mix = |shift: u32| {
        let (c, s) = (((color >> shift) & 0xff) as f32, ((SHELL >> shift) & 0xff) as f32);
        ((s + (c - s) * t).round() as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

/// Four shades of the lane, from barely there to the full color, like the web's graph.
fn shade(count: u64, max: u64, lane: u32) -> u32 {
    if count == 0 || max == 0 {
        return EMPTY;
    }
    let level = ((count * 4).div_ceil(max)).clamp(1, 4);
    blend(lane, [0.32, 0.52, 0.76, 1.0][level as usize - 1])
}

pub fn graph(activity: &Activity, lane: u32, width: usize) -> Vec<Line<'static>> {
    if activity.buckets.is_empty() || width < 20 {
        return vec![];
    }
    match activity.scale.as_str() {
        "year" => year(&activity.buckets, lane, width),
        scale => bars(&activity.buckets, lane, width, scale),
    }
}

/// Weeks as columns, Sunday on top, two days to a cell with half blocks, so each day comes out about square.
fn year(days: &[Bucket], lane: u32, width: usize) -> Vec<Line<'static>> {
    let weeks = days.len().div_ceil(7);
    let first = weeks.saturating_sub(width);
    let max = days.iter().map(|d| d.count).max().unwrap_or(0);
    let color = |index: usize| days.get(index).map(|d| rgb(shade(d.count, max, lane)));

    // Month names over the week each month starts in.
    let mut months = String::new();
    let mut last_month = String::new();
    for week in first..weeks {
        let month = days
            .get(week * 7)
            .map(|d| d.date.get(5..7).unwrap_or("").to_string())
            .unwrap_or_default();
        let column = week - first;
        if month != last_month && months.chars().count() <= column {
            let name = month_name(&month);
            months.push_str(&" ".repeat(column - months.chars().count()));
            if column + name.len() <= width {
                months.push_str(name);
            }
        }
        last_month = month;
    }
    let mut lines = vec![Line::from(Span::styled(months, Theme::faint_style()))];

    for row in 0..4 {
        let mut spans = Vec::with_capacity(weeks - first);
        for week in first..weeks {
            let top = color(week * 7 + row * 2);
            let bottom = if row < 3 { color(week * 7 + row * 2 + 1) } else { None };
            let style = Style::default().fg(top.unwrap_or(rgb(SHELL))).bg(bottom.unwrap_or(rgb(SHELL)));
            spans.push(Span::styled("▀", style));
        }
        lines.push(Line::from(spans));
    }

    let total: u64 = days.iter().map(|d| d.count).sum();
    let mut legend = vec![Span::styled(
        format!("{} commit{} in the last year   ", count(total), if total == 1 { "" } else { "s" }),
        Theme::dim(),
    )];
    legend.push(Span::styled("less ", Theme::faint_style()));
    for level in 0..5u64 {
        let c = if level == 0 { EMPTY } else { shade(level, 4, lane) };
        legend.push(Span::styled("■", Style::default().fg(rgb(c))));
    }
    legend.push(Span::styled(" more", Theme::faint_style()));
    lines.push(Line::from(legend));
    lines
}

fn month_name(month: &str) -> &'static str {
    match month {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => "",
    }
}

/// A newer account's last day (by the hour), week or month, as bars.
fn bars(buckets: &[Bucket], lane: u32, width: usize, scale: &str) -> Vec<Line<'static>> {
    let max = buckets.iter().map(|b| b.count).max().unwrap_or(0);
    let cell = (width / buckets.len()).clamp(1, 4);
    let bar = cell.saturating_sub(1).max(1);
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for b in buckets {
        // Two lines of bars, for sixteen steps.
        let height = if max == 0 { 0 } else { ((b.count * 16).div_ceil(max)) as usize };
        let color = rgb(if b.count == 0 { EMPTY } else { lane });
        let (upper, lower) = if height > 8 {
            (BARS[height - 9], "█")
        } else {
            (" ", if height == 0 { "▁" } else { BARS[height - 1] })
        };
        top.push(Span::styled(upper.repeat(bar), Style::default().fg(color)));
        bottom.push(Span::styled(lower.repeat(bar), Style::default().fg(color)));
        if cell > bar {
            top.push(Span::raw(" ".repeat(cell - bar)));
            bottom.push(Span::raw(" ".repeat(cell - bar)));
        }
    }
    let total: u64 = buckets.iter().map(|b| b.count).sum();
    let period = match scale {
        "day" => "the last 24 hours",
        "week" => "the last 7 days",
        _ => "the last 30 days",
    };
    vec![
        Line::from(top),
        Line::from(bottom),
        Line::from(Span::styled(
            format!("{} commit{} in {period}", count(total), if total == 1 { "" } else { "s" }),
            Theme::dim(),
        )),
    ]
}

/// "uptime 5d · longest 12d · root · level 7 · 1.2k XP ▰▰▰▱▱"
pub fn standing(streaks: Option<&Streaks>, achievements: Option<&Achievements>, lane: u32) -> Option<Line<'static>> {
    let mut spans = Vec::new();
    if let Some(s) = streaks {
        let style = if s.current > 0 {
            Style::default().fg(Theme::lane(0)).add_modifier(Modifier::BOLD)
        } else {
            Theme::dim()
        };
        spans.push(Span::styled(format!("streak {}d", s.current), style));
        spans.push(Span::styled(format!(" · longest {}d", s.longest), Theme::dim()));
    }
    if let Some(a) = achievements.filter(|a| !a.rank.is_empty()) {
        if !spans.is_empty() {
            spans.push(Span::styled("   ", Theme::dim()));
        }
        spans.push(Span::styled(
            a.rank.clone(),
            Style::default().fg(rgb(lane)).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" · level {} · {} XP ", a.level, count(a.xp.max(0) as u64)),
            Theme::dim(),
        ));
        let filled = (a.level_progress.min(100) as usize).div_ceil(20);
        spans.push(Span::styled("▰".repeat(filled), Style::default().fg(rgb(lane))));
        spans.push(Span::styled("▱".repeat(5 - filled), Theme::faint_style()));
    }
    (!spans.is_empty()).then(|| Line::from(spans))
}

/// The unlocked badges, as chips, and how many there are to get.
pub fn badges(achievements: &Achievements, width: usize) -> Option<Line<'static>> {
    if achievements.badges.is_empty() {
        return None;
    }
    let unlocked: Vec<&str> = achievements.badges.iter().filter(|b| b.unlocked).map(|b| b.name.as_str()).collect();
    let mut spans = vec![Span::styled(
        format!("{}/{} ", unlocked.len(), achievements.badges.len()),
        Theme::dim(),
    )];
    let mut used = spans[0].width();
    for name in unlocked {
        let chip = format!(" {name} ");
        if used + chip.chars().count() + 2 > width {
            spans.push(Span::styled("…", Theme::faint_style()));
            break;
        }
        used += chip.chars().count() + 1;
        spans.push(Span::styled(chip, Style::default().fg(Theme::ink()).bg(Theme::wash())));
        spans.push(Span::raw(" "));
    }
    Some(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(n: usize) -> Vec<Bucket> {
        (0..n)
            .map(|i| Bucket {
                date: format!("2026-{:02}-{:02}", 1 + (i / 28) % 12, 1 + i % 28),
                count: (i % 5) as u64,
            })
            .collect()
    }

    #[test]
    fn a_year_is_four_rows_of_weeks() {
        let lines = graph(
            &Activity {
                scale: "year".into(),
                buckets: days(365),
            },
            0x51cf66,
            80,
        );
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[1].width(), 53);
        // Narrower, it keeps the latest weeks.
        assert_eq!(
            graph(
                &Activity {
                    scale: "year".into(),
                    buckets: days(365)
                },
                0x51cf66,
                30
            )[1]
            .width(),
            30
        );
    }

    #[test]
    fn shades_grow_with_the_count() {
        assert_eq!(shade(0, 10, 0xff0000), EMPTY);
        assert_eq!(shade(10, 10, 0xff0000), 0xff0000);
        assert!(shade(1, 10, 0xff0000) >> 16 < shade(9, 10, 0xff0000) >> 16);
    }

    #[test]
    fn newer_accounts_get_bars() {
        let lines = graph(
            &Activity {
                scale: "week".into(),
                buckets: days(7),
            },
            0x51cf66,
            60,
        );
        assert_eq!(lines.len(), 3);
    }
}
