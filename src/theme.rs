//! The web's dark and light palettes (DESIGN.md), downsampled to the 256-color cube on terminals without truecolor.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

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

static LIGHT: AtomicBool = AtomicBool::new(false);

/// Switches every color between the web's dark and light palettes. It takes effect on the next frame.
pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// Asks the terminal for its background color (OSC 11) and says whether it's light. DA1 is asked right after, since
/// every terminal answers that, so one that doesn't know OSC 11 costs no waiting. Raw mode has to be on.
#[cfg(unix)]
pub fn detect_light() -> Option<bool> {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    let mut tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").ok()?;
    tty.write_all(b"\x1b]11;?\x1b\\\x1b[c").ok()?;
    tty.flush().ok()?;

    let deadline = Instant::now() + Duration::from_millis(500);
    let mut answer = Vec::new();
    while !answered(&answer) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut fd = libc::pollfd {
            fd: tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd, for a file that stays open for the call.
        if unsafe { libc::poll(&mut fd, 1, left.as_millis() as libc::c_int) } <= 0 {
            break;
        }
        let mut chunk = [0u8; 256];
        match tty.read(&mut chunk) {
            Ok(n) if n > 0 => answer.extend_from_slice(&chunk[..n]),
            _ => break,
        }
    }
    background(&answer).map(|(r, g, b)| 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.5)
}

#[cfg(not(unix))]
pub fn detect_light() -> Option<bool> {
    None
}

/// DA1's answer, "ESC [ ? ... c", comes last.
#[cfg_attr(not(unix), allow(dead_code))]
fn answered(bytes: &[u8]) -> bool {
    bytes
        .windows(3)
        .position(|w| w == b"\x1b[?")
        .is_some_and(|i| bytes[i..].contains(&b'c'))
}

/// "ESC ] 11 ; rgb:RRRR/GGGG/BBBB", each channel one to four hex digits, as fractions.
#[cfg_attr(not(unix), allow(dead_code))]
fn background(bytes: &[u8]) -> Option<(f32, f32, f32)> {
    let text = String::from_utf8_lossy(bytes);
    let start = text.find("]11;rgb:")? + 8;
    let end = text[start..]
        .find(|c: char| !(c.is_ascii_hexdigit() || c == '/'))
        .map(|e| start + e)
        .unwrap_or(text.len());
    let channel = |hex: &str| -> Option<f32> {
        let max = (16u32.checked_pow(hex.len() as u32)? - 1) as f32;
        Some(u32::from_str_radix(hex, 16).ok()? as f32 / max)
    };
    let mut parts = text[start..end].split('/').map(channel);
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// A color in each palette, as DESIGN.md has it.
fn pick(dark: u32, light: u32) -> u32 {
    if is_light() { light } else { dark }
}

pub struct Theme;

impl Theme {
    pub fn shell_hex() -> u32 {
        pick(0x111112, 0xf6f7f8)
    }
    pub fn wash_hex() -> u32 {
        pick(0x232326, 0xeef0f2)
    }
    pub fn shell() -> Color {
        rgb(Self::shell_hex())
    }
    pub fn popover() -> Color {
        rgb(pick(0x1f1f22, 0xffffff))
    }
    pub fn ink() -> Color {
        rgb(pick(0xececec, 0x121417))
    }
    pub fn muted() -> Color {
        rgb(pick(0xa1a1a6, 0x5b616b))
    }
    pub fn faint() -> Color {
        rgb(pick(0x6b6b70, 0x8d939c))
    }
    pub fn hairline() -> Color {
        rgb(pick(0x2a2a2d, 0xe1e4e8))
    }
    pub fn rail() -> Color {
        rgb(pick(0x36363a, 0xd3d7dd))
    }
    pub fn wash() -> Color {
        rgb(Self::wash_hex())
    }
    pub fn primary() -> Color {
        rgb(pick(0x7aa2f7, 0x3b6fe0))
    }
    pub fn primary_soft() -> Color {
        rgb(pick(0x18233a, 0xe8effc))
    }
    /// A quieter primary, for buttons that aren't the main action.
    pub fn primary_muted() -> Color {
        rgb(pick(0x3b5590, 0xd2def9))
    }
    /// The web's brand red, for types in highlighted code.
    pub fn brand() -> Color {
        rgb(pick(0xff6e5a, 0xd92d1f))
    }
    pub fn destructive() -> Color {
        rgb(pick(0xff6b6b, 0xc92a2a))
    }
    pub fn star() -> Color {
        rgb(pick(0xfcc419, 0xd69e0b))
    }

    pub fn lane(index: u8) -> Color {
        rgb(Self::lane_hex(index))
    }

    pub fn lane_hex(index: u8) -> u32 {
        match index % 6 {
            0 => pick(0xff8a4c, 0xd9480f),
            1 => pick(0x51cf66, 0x2f9e44),
            2 => pick(0xda77f2, 0x9c36b5),
            3 => pick(0x3bc9db, 0x0c8599),
            4 => pick(0xf06595, 0xc2255c),
            _ => pick(0xfcc419, 0xb7791f),
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
        Style::default().fg(rgb(pick(0xe2c08d, 0x8a5a00))).bg(Self::wash())
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
    fn it_reads_the_terminals_background() {
        let white = b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b[?62;22c";
        assert!(answered(white));
        assert_eq!(background(white), Some((1.0, 1.0, 1.0)));
        let dark = b"\x1b]11;rgb:1e/1e/2e\x07\x1b[?1;2c";
        let (r, _, _) = background(dark).unwrap();
        assert!(r < 0.2);
        assert!(background(b"\x1b[?1;2c").is_none());
        assert!(!answered(b"\x1b]11;rgb:0/0/0"));
    }

    #[test]
    fn the_automatic_lane_matches_the_web() {
        // ((0*31 + 'a') * 31 + 'b') = 3105, % 6 = 3, as User::automaticLane computes it.
        assert_eq!(automatic_lane("ab"), 3);
        assert_eq!(automatic_lane(""), 0);
    }
}
