//! The UI's icons: the web's outline icons from Nerd Fonts where the terminal has them, plain symbols elsewhere.
//!
//! Ghostty, kitty and WezTerm ship the Nerd Font symbols themselves, so "auto" picks them there; anywhere else it
//! takes a Nerd Font in Settings, or DEVNULL_ICONS=nerd.

use std::sync::atomic::{AtomicBool, Ordering};

pub struct Icons {
    pub log: &'static str,
    pub notifications: &'static str,
    pub remotes: &'static str,
    pub profile: &'static str,
    pub starred: &'static str,
    pub settings: &'static str,
    pub commit: &'static str,
    pub star: &'static str,
    pub star_full: &'static str,
    pub quote: &'static str,
    pub reply: &'static str,
    pub search: &'static str,
    /// A commit's node on the rail: a ring around a dot.
    pub node: &'static str,
}

/// Lucide's icons on the web, as near as Nerd Fonts come: Material's vertical commit, and Codicons for the rest.
const NERD: Icons = Icons {
    log: "\u{f0718}",
    notifications: "\u{eaa2}",
    remotes: "\u{eac7}",
    profile: "\u{ea67}",
    starred: "\u{ea6a}",
    settings: "\u{eb52}",
    commit: "\u{ea73}",
    star: "\u{ea6a}",
    star_full: "\u{eb59}",
    quote: "\u{eb33}",
    reply: "\u{ea7d}",
    search: "\u{ea6d}",
    node: "\u{f043e}",
};

/// Symbols that fonts draw one cell wide, so they line up. Iosevka and its kin draw most geometric shapes (◉ ◔ ◯)
/// two cells wide, and the terminal lets them spill.
const PLAIN: Icons = Icons {
    log: "⦿",
    notifications: "⍾",
    remotes: "✉",
    profile: "☺",
    starred: "☆",
    settings: "≡",
    commit: "⊕",
    star: "☆",
    star_full: "★",
    quote: "❝",
    reply: "↩",
    search: "◎",
    node: "⦿",
};

static NERD_ON: AtomicBool = AtomicBool::new(false);

/// Applies the setting: "nerd", "plain", or "auto". DEVNULL_ICONS wins over it.
pub fn set(setting: &str) {
    let setting = std::env::var("DEVNULL_ICONS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| setting.to_string())
        .to_lowercase();
    let nerd = match setting.as_str() {
        "nerd" => true,
        "plain" => false,
        _ => bundles_nerd_symbols(),
    };
    NERD_ON.store(nerd, Ordering::Relaxed);
}

fn bundles_nerd_symbols() -> bool {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    matches!(env("TERM_PROGRAM").as_str(), "ghostty" | "WezTerm")
        || matches!(env("TERM").as_str(), "xterm-ghostty" | "xterm-kitty" | "wezterm")
        || !env("KITTY_WINDOW_ID").is_empty()
        || !env("GHOSTTY_RESOURCES_DIR").is_empty()
}

pub fn nerd() -> bool {
    NERD_ON.load(Ordering::Relaxed)
}

pub fn get() -> &'static Icons {
    if nerd() { &NERD } else { &PLAIN }
}

/// What follows an icon. A Nerd icon gets a plain space, which terminals let it grow into, as its outline is drawn
/// for two cells; a symbol gets a no-break space, which keeps it to its own cell.
pub fn gap() -> &'static str {
    if nerd() { " " } else { "\u{a0}" }
}
