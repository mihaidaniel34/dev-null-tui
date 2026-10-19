//! The web's dark palette (DESIGN.md), downsampled to the 256-color cube on terminals without truecolor.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use serde_json::Value;

use crate::model::User;

fn truecolor() -> bool {
    static TRUECOLOR: OnceLock<bool> = OnceLock::new();
    *TRUECOLOR.get_or_init(|| {
        let colorterm = std::env::var("COLORTERM").unwrap_or_default().to_lowercase();
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();
        colorterm.contains("truecolor")
            || colorterm.contains("24bit")
            || matches!(term_program.as_str(), "iTerm.app" | "WezTerm" | "vscode" | "ghostty" | "Hyper")
            || std::env::var("KITTY_WINDOW_ID").is_ok()
    })
}

pub fn rgb(hex: u32) -> Color {
    let (r, g, b) = ((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    if truecolor() {
        return Color::Rgb(r, g, b);
    }

    // Grays go to the 24-step ramp, everything else to the 6x6x6 cube.
    if r.abs_diff(g) < 10 && g.abs_diff(b) < 10 {
        let level = (r as u16 + g as u16 + b as u16) / 3;
        if level < 8 {
            return Color::Indexed(16);
        }
        if level > 238 {
            return Color::Indexed(231);
        }
        return Color::Indexed(232 + ((level - 8) / 10).min(23) as u8);
    }

    let cube = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((v - 35) / 40).min(5)
        }
    };
    Color::Indexed(16 + 36 * cube(r) + 6 * cube(g) + cube(b))
}

pub struct Theme;

impl Theme {
    pub fn shell() -> Color {
        rgb(0x111112)
    }
    pub fn popover() -> Color {
        rgb(0x1f1f22)
    }
    pub fn ink() -> Color {
        rgb(0xececec)
    }
    pub fn muted() -> Color {
        rgb(0xa1a1a6)
    }
    pub fn faint() -> Color {
        rgb(0x6b6b70)
    }
    pub fn hairline() -> Color {
        rgb(0x2a2a2d)
    }
    pub fn rail() -> Color {
        rgb(0x36363a)
    }
    pub fn wash() -> Color {
        rgb(0x232326)
    }
    pub fn primary() -> Color {
        rgb(0x7aa2f7)
    }
    pub fn primary_soft() -> Color {
        rgb(0x18233a)
    }
    pub fn destructive() -> Color {
        rgb(0xff6b6b)
    }
    pub fn star() -> Color {
        rgb(0xfcc419)
    }

    pub fn lane(index: u8) -> Color {
        rgb(Self::lane_hex(index))
    }

    pub fn lane_hex(index: u8) -> u32 {
        match index % 6 {
            0 => 0xff8a4c,
            1 => 0x51cf66,
            2 => 0xda77f2,
            3 => 0x3bc9db,
            4 => 0xf06595,
            _ => 0xfcc419,
        }
    }

    /// The color the web draws a user in: their chosen lane or custom color, or the same hash of their id the web
    /// falls back to (User::automaticLane), which is also what the public API leaves out.
    pub fn user_lane(user: &User) -> Color {
        rgb(Self::user_lane_hex(user))
    }

    pub fn user_lane_hex(user: &User) -> u32 {
        match &user.lane {
            Some(Value::Number(n)) => Self::lane_hex(n.as_u64().unwrap_or(0) as u8),
            Some(Value::String(hex)) if hex.len() == 7 && hex.starts_with('#') => {
                u32::from_str_radix(&hex[1..], 16).unwrap_or_else(|_| Self::lane_hex(automatic_lane(&user.id)))
            }
            _ => Self::lane_hex(automatic_lane(&user.id)),
        }
    }

    pub fn text() -> Style {
        Style::default().fg(Self::ink())
    }
    pub fn dim() -> Style {
        Style::default().fg(Self::muted())
    }
    pub fn faint_style() -> Style {
        Style::default().fg(Self::faint())
    }
    pub fn bold() -> Style {
        Style::default().fg(Self::ink()).add_modifier(Modifier::BOLD)
    }
    pub fn link() -> Style {
        Style::default().fg(Self::primary())
    }
    pub fn code() -> Style {
        Style::default().fg(rgb(0xe2c08d)).bg(Self::wash())
    }
}

pub fn automatic_lane(id: &str) -> u8 {
    let mut hash: u64 = 0;
    for character in id.chars() {
        hash = (hash * 31 + character as u64) & 0xFFFF_FFFF;
    }
    (hash % 6) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_automatic_lane_matches_the_web() {
        // ((0*31 + 'a') * 31 + 'b') = 3105, % 6 = 3, as User::automaticLane computes it.
        assert_eq!(automatic_lane("ab"), 3);
        assert_eq!(automatic_lane(""), 0);
    }
}
