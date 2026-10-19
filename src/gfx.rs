//! Avatars and commit images, on terminals that speak the kitty graphics protocol (kitty, Ghostty).
//!
//! Each picture is sent to the terminal once, as a virtual placement, and then drawn with Unicode placeholders: text
//! cells whose foreground color names the image and whose diacritics name the row and column. So pictures are plain
//! text to ratatui, and scroll, highlight and redraw like everything around them. Elsewhere nothing is sent, and the
//! UI keeps its glyphs.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine;
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage, imageops::FilterType};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use tokio::sync::Semaphore;

/// The row and column diacritics, as kitty writes them; images are kept within this many cells a side.
const DIACRITICS: [char; 40] = [
    '\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}', '\u{0312}', '\u{033D}', '\u{033E}', '\u{033F}', '\u{0346}', '\u{034A}', '\u{034B}',
    '\u{034C}', '\u{0350}', '\u{0351}', '\u{0352}', '\u{0357}', '\u{035B}', '\u{0363}', '\u{0364}', '\u{0365}', '\u{0366}', '\u{0367}',
    '\u{0368}', '\u{0369}', '\u{036A}', '\u{036B}', '\u{036C}', '\u{036D}', '\u{036E}', '\u{036F}', '\u{0483}', '\u{0484}', '\u{0485}',
    '\u{0486}', '\u{0487}', '\u{0592}', '\u{0593}', '\u{0594}', '\u{0595}', '\u{0597}',
];
pub const MAX_CELLS: u16 = DIACRITICS.len() as u16;
const PLACEHOLDER: char = '\u{10EEEE}';

/// A picture at a size, in cells. An avatar is cut round, with a ring in its owner's lane color.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub url: String,
    pub cols: u16,
    pub rows: u16,
    pub ring: Option<u32>,
}

enum State {
    Loading,
    Ready(u32),
    Failed,
}

struct Store {
    states: HashMap<Key, State>,
    queue: Vec<Key>,
    next_id: u32,
    sent: Vec<u32>,
}

struct Support {
    /// Inside tmux, escapes are wrapped so they pass through to the terminal.
    tmux: bool,
}

static SUPPORT: OnceLock<Option<Support>> = OnceLock::new();
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
static DOWNLOADS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn store() -> std::sync::MutexGuard<'static, Store> {
    STORE
        .get_or_init(|| {
            // Ids live in the 24-bit foreground color; a random start keeps them clear of other programs' images.
            let start = 0x10000 + (ssh_key::rand_core::RngCore::next_u32(&mut ssh_key::rand_core::OsRng) % 0x700000);
            Mutex::new(Store {
                states: HashMap::new(),
                queue: Vec::new(),
                next_id: start,
                sent: Vec::new(),
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Decides once whether this terminal gets pictures. `DEVNULL_IMAGES=off` turns them off, `=on` forces them on.
pub fn init() {
    SUPPORT.get_or_init(|| {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        match env("DEVNULL_IMAGES").to_lowercase().as_str() {
            "off" | "0" | "no" | "false" => return None,
            "on" | "1" | "yes" | "true" | "kitty" => {
                return Some(Support {
                    tmux: !env("TMUX").is_empty(),
                });
            }
            _ => {}
        }
        let kitty = !env("KITTY_WINDOW_ID").is_empty() || env("TERM") == "xterm-kitty";
        let ghostty = env("TERM_PROGRAM") == "ghostty" || env("TERM") == "xterm-ghostty" || !env("GHOSTTY_RESOURCES_DIR").is_empty();
        if !kitty && !ghostty {
            return None;
        }
        let tmux = !env("TMUX").is_empty();
        if tmux {
            // Without passthrough, tmux would swallow the images and leave the placeholders as boxes.
            let passthrough = std::process::Command::new("tmux")
                .args(["show", "-gv", "allow-passthrough"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            if passthrough != "on" && passthrough != "all" {
                return None;
            }
        }
        Some(Support { tmux })
    });
}

pub fn enabled() -> bool {
    SUPPORT.get().is_some_and(Option::is_some)
}

/// One row of a picture, as placeholder cells, once it's on the terminal. Until then it's queued, and None.
pub fn row(key: &Key, row: u16) -> Option<Span<'static>> {
    if !enabled() || key.url.is_empty() || key.cols == 0 || key.rows == 0 || key.cols > MAX_CELLS || key.rows > MAX_CELLS || row >= key.rows
    {
        return None;
    }
    let mut store = store();
    match store.states.get(key) {
        Some(State::Ready(id)) => {
            let id = *id;
            let mut cells = String::with_capacity(key.cols as usize * 8);
            for col in 0..key.cols {
                cells.push(PLACEHOLDER);
                cells.push(DIACRITICS[row as usize]);
                cells.push(DIACRITICS[col as usize]);
            }
            Some(Span::styled(
                cells,
                Style::default().fg(Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8)),
            ))
        }
        Some(_) => None,
        None => {
            store.states.insert(key.clone(), State::Loading);
            store.queue.push(key.clone());
            None
        }
    }
}

pub fn take_queue() -> Vec<Key> {
    std::mem::take(&mut store().queue)
}

/// Sends a processed picture to the terminal. Called between frames, from the main loop.
pub fn loaded(key: Key, png: Option<Vec<u8>>) {
    let Some(support) = SUPPORT.get().and_then(Option::as_ref) else {
        return;
    };
    let mut store = store();
    let Some(png) = png else {
        store.states.insert(key, State::Failed);
        return;
    };
    let id = store.next_id;
    store.next_id += 1;
    store.sent.push(id);
    store.states.insert(key.clone(), State::Ready(id));
    drop(store);

    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    let chunks: Vec<&[u8]> = encoded.as_bytes().chunks(4096).collect();
    let mut out = Vec::with_capacity(encoded.len() + chunks.len() * 64);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let head = if i == 0 {
            format!("a=T,U=1,f=100,q=2,i={id},c={},r={},m={more}", key.cols, key.rows)
        } else {
            format!("m={more}")
        };
        out.extend(wrap(
            support,
            &format!("\x1b_G{head};{}\x1b\\", std::str::from_utf8(chunk).unwrap_or("")),
        ));
    }
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&out);
    let _ = stdout.flush();
}

/// Frees what was sent, on the way out.
pub fn cleanup() {
    let Some(support) = SUPPORT.get().and_then(Option::as_ref) else {
        return;
    };
    let ids = std::mem::take(&mut store().sent);
    let mut stdout = std::io::stdout().lock();
    for id in ids {
        let _ = stdout.write_all(&wrap(support, &format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\")));
    }
    let _ = stdout.flush();
}

fn wrap(support: &Support, escape: &str) -> Vec<u8> {
    if support.tmux {
        format!("\x1bPtmux;{}\x1b\\", escape.replace('\x1b', "\x1b\x1b")).into_bytes()
    } else {
        escape.as_bytes().to_vec()
    }
}

/// Downloads and prepares a picture. None when it can't be had, so the UI keeps its glyph.
pub async fn fetch(key: Key) -> Option<Vec<u8>> {
    // Unlike the API's client, this one follows redirects, which image hosts use.
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    let http = HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("devnull-tui/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("HTTP client")
    });
    let permits = DOWNLOADS.get_or_init(|| Arc::new(Semaphore::new(8))).clone();
    let _permit = permits.acquire_owned().await.ok()?;
    let response = http.get(&key.url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    tokio::task::spawn_blocking(move || process(&bytes, &key)).await.ok().flatten()
}

/// GitHub serves avatars at any size; this asks for one fit for a few cells.
pub fn avatar_url(url: &str) -> String {
    if url.contains("avatars.githubusercontent.com") && !url.contains("s=") {
        format!("{url}{}s=96", if url.contains('?') { '&' } else { '?' })
    } else {
        url.to_string()
    }
}

fn process(bytes: &[u8], key: &Key) -> Option<Vec<u8>> {
    let format = image::guess_format(bytes).ok()?;
    let picture = image::load_from_memory_with_format(bytes, format).ok()?;
    let out = match key.ring {
        Some(ring) => avatar(&picture, ring),
        None => photo(&picture, key),
    };
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png).ok()?;
    Some(png)
}

/// Cut round, inside a ring in the owner's lane color, as the web draws avatars.
fn avatar(picture: &DynamicImage, ring: u32) -> RgbaImage {
    const SIZE: u32 = 96;
    let (w, h) = picture.dimensions();
    let side = w.min(h);
    let square = picture
        .crop_imm((w - side) / 2, (h - side) / 2, side, side)
        .resize_exact(SIZE, SIZE, FilterType::Lanczos3)
        .to_rgba8();
    let ring = Rgba([(ring >> 16) as u8, (ring >> 8) as u8, ring as u8, 255]);
    let center = SIZE as f32 / 2.0;
    let (outer, ring_in, photo_out) = (center - 0.5, center - 5.0, center - 8.0);

    RgbaImage::from_fn(SIZE, SIZE, |x, y| {
        let d = ((x as f32 + 0.5 - center).powi(2) + (y as f32 + 0.5 - center).powi(2)).sqrt();
        let coverage = |edge: f32| (edge - d + 0.5).clamp(0.0, 1.0);
        if d <= photo_out + 0.5 {
            let mut p = *square.get_pixel(x, y);
            p[3] = (p[3] as f32 * coverage(photo_out)) as u8;
            p
        } else if d >= ring_in - 0.5 {
            let alpha = coverage(outer) * (d - ring_in + 0.5).clamp(0.0, 1.0);
            Rgba([ring[0], ring[1], ring[2], (alpha * 255.0) as u8])
        } else {
            Rgba([0, 0, 0, 0])
        }
    })
}

/// Scaled to fit its cells, with rounded corners.
fn photo(picture: &DynamicImage, key: &Key) -> RgbaImage {
    // About two pixels per point on a retina screen, so it stays sharp.
    let (max_w, max_h) = (key.cols as u32 * 20, key.rows as u32 * 40);
    let mut img = picture.resize(max_w, max_h, FilterType::Lanczos3).to_rgba8();
    let (w, h) = img.dimensions();
    let radius = (w.min(h) as f32 * 0.06).clamp(4.0, 18.0);
    for y in 0..h {
        for x in 0..w {
            let cx = if (x as f32) < radius {
                radius
            } else if (x as f32) > w as f32 - radius {
                w as f32 - radius
            } else {
                continue;
            };
            let cy = if (y as f32) < radius {
                radius
            } else if (y as f32) > h as f32 - radius {
                h as f32 - radius
            } else {
                continue;
            };
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let p = img.get_pixel_mut(x, y);
            p[3] = (p[3] as f32 * (radius - d + 0.5).clamp(0.0, 1.0)) as u8;
        }
    }
    img
}

/// How many cells a picture of this size takes at most `max_cols` wide and `max_rows` tall, with cells about
/// twice as tall as they're wide.
pub fn fit(width: u64, height: u64, max_cols: u16, max_rows: u16) -> (u16, u16) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let max_cols = max_cols.min(MAX_CELLS) as f64;
    let max_rows = max_rows.min(MAX_CELLS) as f64;
    let aspect = height as f64 / width as f64;
    let mut cols = max_cols;
    let mut rows = (cols * aspect / 2.0).round();
    if rows > max_rows {
        rows = max_rows;
        cols = (rows * 2.0 / aspect).round().min(max_cols);
    }
    (cols.max(1.0) as u16, rows.max(1.0) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fits_pictures_into_cells() {
        assert_eq!(fit(1600, 900, 40, 12), (40, 11));
        assert_eq!(fit(900, 1600, 40, 12), (14, 12));
        assert_eq!(fit(0, 10, 40, 12), (0, 0));
    }

    #[test]
    fn avatars_are_round() {
        let img = avatar(&DynamicImage::new_rgb8(10, 10), 0xff8a4c);
        assert_eq!(img.get_pixel(0, 0)[3], 0);
        assert_eq!(img.get_pixel(48, 48)[3], 255);
        assert_eq!(img.get_pixel(48, 1).0[..3], [0xff, 0x8a, 0x4c]);
    }
}
