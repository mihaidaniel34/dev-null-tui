//! Themes: the web's dark and light palettes (DESIGN.md), Windows 98 and whatever else is in themes/, as TOML files,
//! plus your own from the config dir. Colors are downsampled to the 256-color cube on terminals without truecolor.

use std::fs;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result, anyhow, bail};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::BorderType;
use serde::Deserialize;
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

// Themes -------------------------------------------------------------------------------------------------------

/// The themes that come with devnull, in the order `t` goes through them. Adding one is a TOML file in themes/ and
/// a line here.
const BUILT_IN: &[(&str, &str)] = &[
    ("dark", include_str!("../themes/dark.toml")),
    ("light", include_str!("../themes/light.toml")),
    ("windows-98", include_str!("../themes/windows-98.toml")),
];

/// "#c0c0c0", "c0c0c0" or "#ccc", as 0xRRGGBB.
pub fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok(),
        3 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            Some(((v >> 8) & 0xf) * 0x110000 + ((v >> 4) & 0xf) * 0x1100 + (v & 0xf) * 0x11)
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Hex(u32);

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        parse_hex(&text)
            .map(Hex)
            .ok_or_else(|| serde::de::Error::custom(format!("{text:?} isn't a color like \"#c0c0c0\"")))
    }
}

macro_rules! colors {
    ($($name:ident),* $(,)?) => {
        /// A theme's colors, as 0xRRGGBB.
        #[derive(Clone, Copy, Default)]
        struct Colors {
            $($name: u32,)*
        }

        /// The colors a theme file sets. The rest come from the theme it extends.
        #[derive(Default, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ColorsFile {
            $($name: Option<Hex>,)*
        }

        impl ColorsFile {
            fn over(&self, base: Colors) -> Colors {
                Colors {
                    $($name: self.$name.map_or(base.$name, |h| h.0),)*
                }
            }

            fn missing(&self) -> Vec<&'static str> {
                let mut missing = Vec::new();
                $(if self.$name.is_none() {
                    missing.push(stringify!($name));
                })*
                missing
            }
        }
    };
}

colors!(
    shell,
    popover,
    ink,
    muted,
    faint,
    hairline,
    rail,
    wash,
    highlight,
    primary,
    on_primary,
    primary_soft,
    on_primary_soft,
    primary_muted,
    brand,
    destructive,
    star,
    code,
);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    name: Option<String>,
    extends: Option<String>,
    borders: Option<Borders>,
    #[serde(default)]
    colors: ColorsFile,
    lanes: Option<[Hex; 6]>,
    title_bar: Option<TitleBarFile>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Borders {
    Rounded,
    Plain,
    Thick,
    Double,
}

/// One color, or two to fade between from left to right.
#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
enum FadeFile {
    One(Hex),
    Two([Hex; 2]),
}

impl FadeFile {
    fn ends(self) -> (u32, u32) {
        match self {
            FadeFile::One(c) => (c.0, c.0),
            FadeFile::Two([a, b]) => (a.0, b.0),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TitleBarFile {
    active: FadeFile,
    active_ink: Hex,
    inactive: Option<FadeFile>,
    inactive_ink: Option<Hex>,
}

/// Commit headers drawn as window title bars.
#[derive(Clone, Copy)]
pub struct TitleBar {
    /// The selected commit's, fading from the first color to the second, and its text.
    pub active: (u32, u32),
    pub active_ink: u32,
    /// Everyone else's, when the theme has one.
    pub inactive: Option<((u32, u32), u32)>,
}

#[derive(Clone)]
pub struct Palette {
    pub name: String,
    colors: Colors,
    lanes: [u32; 6],
    borders: BorderType,
    title_bar: Option<TitleBar>,
}

/// Where your own themes go: `themes/` in the config dir, one TOML file each, named after the theme's id.
pub fn custom_dir() -> PathBuf {
    crate::config::dir().join("themes")
}

/// A theme by its id, reading your themes folder first, so a file there can also change a built-in theme.
pub fn load(id: &str) -> Result<Palette> {
    resolve(id, false, 0)
}

fn resolve(id: &str, built_in_only: bool, depth: usize) -> Result<Palette> {
    if depth > 8 {
        bail!("The theme {id} goes round in circles with extends.");
    }
    let path = custom_dir().join(format!("{id}.toml"));
    let custom = !built_in_only && path.is_file();
    let built_in = BUILT_IN.iter().find(|(i, _)| *i == id).map(|(_, text)| *text);
    let text = if custom {
        fs::read_to_string(&path).with_context(|| format!("Could not read {}.", path.display()))?
    } else {
        built_in.with_context(|| format!("There's no theme called {id}."))?.to_string()
    };
    let file: File = toml::from_str(&text).map_err(|e| {
        let line = e.span().map(|s| text[..s.start].matches('\n').count() + 1);
        // serde lists every field it expected, which is too long to read in a toast.
        let message = e.message();
        let message = message.split(", expected one of").next().unwrap_or(message);
        anyhow!("{id}.toml{}: {message}", line.map(|l| format!(" line {l}")).unwrap_or_default())
    })?;

    // A theme starts from the one it extends, or from dark. A file of yours named after a built-in theme starts from
    // that theme.
    let base = match (&file.extends, custom && built_in.is_some()) {
        (Some(parent), _) if parent == id => Some(resolve(id, true, depth + 1)?),
        (Some(parent), _) => Some(resolve(parent, false, depth + 1)?),
        (None, true) => Some(resolve(id, true, depth + 1)?),
        (None, false) if id != "dark" => Some(resolve("dark", false, depth + 1)?),
        (None, false) => None,
    };
    if base.is_none() {
        let mut missing = file.colors.missing();
        if file.lanes.is_none() {
            missing.push("lanes");
        }
        if !missing.is_empty() {
            bail!(
                "{id}.toml extends nothing, so it needs every color. Missing: {}.",
                missing.join(", ")
            );
        }
    }

    Ok(Palette {
        name: file.name.clone().unwrap_or_else(|| id.to_string()),
        colors: file.colors.over(base.as_ref().map(|b| b.colors).unwrap_or_default()),
        lanes: file
            .lanes
            .map(|lanes| lanes.map(|h| h.0))
            .or(base.as_ref().map(|b| b.lanes))
            .unwrap_or_default(),
        borders: match file.borders {
            Some(Borders::Rounded) => BorderType::Rounded,
            Some(Borders::Plain) => BorderType::Plain,
            Some(Borders::Thick) => BorderType::Thick,
            Some(Borders::Double) => BorderType::Double,
            None => base.as_ref().map_or(BorderType::Rounded, |b| b.borders),
        },
        title_bar: match file.title_bar {
            Some(t) => Some(TitleBar {
                active: t.active.ends(),
                active_ink: t.active_ink.0,
                inactive: t.inactive.map(|f| (f.ends(), t.inactive_ink.unwrap_or(t.active_ink).0)),
            }),
            None => base.as_ref().and_then(|b| b.title_bar),
        },
    })
}

/// The built-in dark theme, for when the one asked for doesn't load.
pub fn fallback() -> Palette {
    resolve("dark", true, 0).expect("the built-in dark theme loads")
}

fn current() -> &'static RwLock<Palette> {
    static CURRENT: OnceLock<RwLock<Palette>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new(fallback()))
}

/// Switches every color to this theme's. It takes effect on the next frame.
pub fn set(palette: Palette) {
    *current().write().unwrap_or_else(|e| e.into_inner()) = palette;
}

fn read<T>(f: impl FnOnce(&Palette) -> T) -> T {
    f(&current().read().unwrap_or_else(|e| e.into_inner()))
}

fn colors() -> Colors {
    read(|p| p.colors)
}

/// The color a fraction of the way from one to the other.
pub fn fade(from: u32, to: u32, t: f32) -> u32 {
    let mix = |shift: u32| {
        let (a, b) = (((from >> shift) & 0xff) as f32, ((to >> shift) & 0xff) as f32);
        ((a + (b - a) * t.clamp(0.0, 1.0)).round() as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

pub struct Theme;

impl Theme {
    /// The theme's name, as Settings shows it.
    pub fn name() -> String {
        read(|p| p.name.clone())
    }
    pub fn borders() -> BorderType {
        read(|p| p.borders)
    }
    pub fn title_bar() -> Option<TitleBar> {
        read(|p| p.title_bar)
    }

    pub fn shell_hex() -> u32 {
        colors().shell
    }
    pub fn wash_hex() -> u32 {
        colors().wash
    }
    pub fn shell() -> Color {
        rgb(colors().shell)
    }
    pub fn popover() -> Color {
        rgb(colors().popover)
    }
    pub fn ink() -> Color {
        rgb(colors().ink)
    }
    pub fn muted() -> Color {
        rgb(colors().muted)
    }
    pub fn faint() -> Color {
        rgb(colors().faint)
    }
    pub fn hairline() -> Color {
        rgb(colors().hairline)
    }
    pub fn rail() -> Color {
        rgb(colors().rail)
    }
    pub fn wash() -> Color {
        rgb(colors().wash)
    }
    /// Behind the selected commit.
    pub fn highlight() -> Color {
        rgb(colors().highlight)
    }
    pub fn primary() -> Color {
        rgb(colors().primary)
    }
    /// Text on a primary button.
    pub fn on_primary() -> Color {
        rgb(colors().on_primary)
    }
    pub fn primary_soft() -> Color {
        rgb(colors().primary_soft)
    }
    /// Text on primary_soft, like the page you're on in the sidebar.
    pub fn on_primary_soft() -> Color {
        rgb(colors().on_primary_soft)
    }
    /// A quieter primary, for buttons that aren't the main action.
    pub fn primary_muted() -> Color {
        rgb(colors().primary_muted)
    }
    /// The web's brand red, for types in highlighted code.
    pub fn brand() -> Color {
        rgb(colors().brand)
    }
    pub fn destructive() -> Color {
        rgb(colors().destructive)
    }
    pub fn star() -> Color {
        rgb(colors().star)
    }

    pub fn lane(index: u8) -> Color {
        rgb(Self::lane_hex(index))
    }

    pub fn lane_hex(index: u8) -> u32 {
        read(|p| p.lanes[index as usize % 6])
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
                parse_hex(hex).unwrap_or_else(|| Self::lane_hex(automatic_lane(&user.id)))
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
        Style::default().fg(rgb(colors().code)).bg(Self::wash())
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
    fn every_built_in_theme_loads() {
        for (id, _) in BUILT_IN {
            let palette = resolve(id, true, 0).unwrap_or_else(|e| panic!("{id}: {e:#}"));
            assert!(!palette.name.is_empty());
        }
        let w98 = resolve("windows-98", true, 0).unwrap();
        assert_eq!(w98.colors.shell, 0xc0c0c0);
        assert_eq!(w98.borders, BorderType::Plain);
        assert!(w98.title_bar.is_some_and(|t| t.active == (0x000080, 0x1084d0)));
    }

    #[test]
    fn a_theme_only_sets_what_differs() {
        let file: File = toml::from_str("extends = \"light\"\n[colors]\nprimary = \"#f0a\"\n").unwrap();
        assert_eq!(file.colors.over(resolve("light", true, 0).unwrap().colors).primary, 0xff00aa);
        assert!(toml::from_str::<File>("[colors]\nprimry = \"#fff\"\n").is_err());
        assert!(toml::from_str::<File>("[colors]\nink = \"white\"\n").is_err());
        assert_eq!(parse_hex("#C0C0C0"), Some(0xc0c0c0));
        assert_eq!(parse_hex("+12345"), None);
    }

    #[test]
    fn the_automatic_lane_matches_the_web() {
        // ((0*31 + 'a') * 31 + 'b') = 3105, % 6 = 3, as User::automaticLane computes it.
        assert_eq!(automatic_lane("ab"), 3);
        assert_eq!(automatic_lane(""), 0);
    }
}
