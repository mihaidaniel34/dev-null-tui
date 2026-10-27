//! Text larger than the terminal's font, on terminals that speak kitty's text sizing protocol (OSC 66, kitty 0.40+).
//!
//! The UI lays big text out as two rows of marker cells: the first names the text, the rest only hold its place.
//! Once a frame is drawn, `finish` turns each intact block into one cell whose symbol is the escape and whose width
//! is forced to the block's, and has ratatui skip the row below, which the terminal fills. A block an overlay or the
//! edge of the screen cut into goes back to plain text. Elsewhere no markers are made, and text keeps its size.

use std::cell::RefCell;
use std::num::NonZeroU16;
use std::sync::OnceLock;

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::style::Style;
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

use crate::text::width;

/// How big. Both take two rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    /// Twice the font, for headings.
    Double,
    /// One and a half times, sitting on the second row's baseline, for names.
    Large,
}

/// Where a block starts: this plus its index in the frame's list.
const FIRST: u32 = 0xE000;
const LAST: u32 = 0xF7FF;
/// Every other cell of a block.
const PAD: char = '\u{F8FE}';

struct Block {
    text: String,
    size: Size,
    width: u16,
}

thread_local! {
    static BLOCKS: RefCell<Vec<Block>> = const { RefCell::new(Vec::new()) };
}

static SUPPORTED: OnceLock<bool> = OnceLock::new();

/// Asks the terminal whether it scales text, by printing a space at twice the size between two cursor reports.
/// `DEVNULL_BIG_TEXT=off` skips it, `=on` assumes yes. Raw mode has to be on.
pub fn detect() {
    SUPPORTED.get_or_init(
        || match std::env::var("DEVNULL_BIG_TEXT").unwrap_or_default().to_lowercase().as_str() {
            "off" | "0" | "no" | "false" => false,
            "on" | "1" | "yes" | "true" => true,
            // tmux keeps its own grid, and would print the text at one size.
            _ if std::env::var("TMUX").is_ok_and(|t| !t.is_empty()) => false,
            _ => probe().unwrap_or(false),
        },
    );
}

pub fn enabled() -> bool {
    SUPPORTED.get().copied().unwrap_or(false)
}

#[cfg(unix)]
fn probe() -> Option<bool> {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    let mut tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").ok()?;
    tty.write_all(b"\r\x1b[6n\x1b]66;s=2; \x07\x1b[6n").ok()?;
    tty.flush().ok()?;

    let deadline = Instant::now() + Duration::from_millis(500);
    let mut answer = Vec::new();
    while columns(&answer).len() < 2 {
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
    // The space may have left a two-row cell behind; the first frame is drawn over a clean screen.
    let _ = tty.write_all(b"\r\x1b[2J");
    let _ = tty.flush();
    match columns(&answer)[..] {
        [before, after, ..] => Some(after == before + 2),
        _ => None,
    }
}

#[cfg(not(unix))]
fn probe() -> Option<bool> {
    None
}

/// The columns of each cursor report, "ESC [ row ; col R", in order.
fn columns(bytes: &[u8]) -> Vec<u16> {
    let text = String::from_utf8_lossy(bytes);
    text.split("\x1b[")
        .filter_map(|part| {
            let report = &part[..part.find('R')?];
            let (_, col) = report.split_once(';')?;
            col.parse().ok()
        })
        .collect()
}

/// Forgets the last frame's blocks. Called before each frame is laid out.
pub fn reset() {
    BLOCKS.with(|b| b.borrow_mut().clear());
}

/// `text` at `size`, as its two rows and their width in cells. None when the terminal draws one size, or the text
/// can't be scaled, and the caller lays it out as usual.
pub fn rows(text: &str, size: Size, style: Style) -> Option<(Span<'static>, Span<'static>, usize)> {
    if !enabled() || text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    let cells = match size {
        Size::Double => 2 * width(text),
        // Halves don't fit wide characters' cells.
        Size::Large if text.chars().all(|c| c.width() == Some(1)) => large_chunks(text).map(|(_, w)| 2 * w).sum(),
        Size::Large => return None,
    };
    if cells < 2 || cells > u16::MAX as usize {
        return None;
    }
    let id = BLOCKS.with(|b| {
        let mut blocks = b.borrow_mut();
        let id = FIRST + blocks.len() as u32;
        (id <= LAST).then(|| {
            blocks.push(Block {
                text: text.to_string(),
                size,
                width: cells as u16,
            });
            id
        })
    })?;
    let mut top = char::from_u32(id)?.to_string();
    top.extend(std::iter::repeat_n(PAD, cells - 1));
    let bottom: String = std::iter::repeat_n(PAD, cells).collect();
    Some((Span::styled(top, style), Span::styled(bottom, style), cells))
}

/// One and a half times the font is one and a half cells a character, so four characters fill a three-cell-wide
/// block, and the last few the narrowest block they fit.
fn large_chunks(text: &str) -> impl Iterator<Item = (String, usize)> + '_ {
    let chars: Vec<char> = text.chars().collect();
    let chunks: Vec<(String, usize)> = chars
        .chunks(4)
        .map(|c| (c.iter().collect::<String>(), (3 * c.len()).div_ceil(4)))
        .collect();
    chunks.into_iter()
}

fn escape(block: &Block) -> String {
    match block.size {
        Size::Double => format!("\x1b]66;s=2;{}\x07", block.text),
        Size::Large => large_chunks(&block.text)
            .map(|(chunk, w)| format!("\x1b]66;s=2:w={w}:n=3:d=4:v=1;{chunk}\x07"))
            .collect(),
    }
}

/// Turns the frame's marker blocks into big text, or back into plain text where something cut into them.
pub fn finish(buf: &mut Buffer) {
    let blocks = BLOCKS.with(|b| std::mem::take(&mut *b.borrow_mut()));
    if blocks.is_empty() {
        return;
    }
    let area = buf.area;
    let is_pad = |buf: &Buffer, x: u16, y: u16| buf.cell((x, y)).is_some_and(|c| c.symbol().starts_with(PAD));
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(id) = buf[(x, y)].symbol().chars().next().map(u32::from) else {
                continue;
            };
            let Some(block) = (FIRST..=LAST).contains(&id).then(|| blocks.get((id - FIRST) as usize)).flatten() else {
                continue;
            };
            let w = block.width;
            let intact = x + w <= area.right()
                && y + 1 < area.bottom()
                && (x + 1..x + w).all(|cx| is_pad(buf, cx, y))
                && (x..x + w).all(|cx| is_pad(buf, cx, y + 1));
            if intact {
                buf[(x, y)]
                    .set_symbol(&escape(block))
                    .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(w).expect("two cells or more")));
                for cx in x + 1..x + w {
                    buf[(cx, y)].set_symbol(" ");
                }
                for cx in x..x + w {
                    buf[(cx, y + 1)].set_symbol(" ").set_diff_option(CellDiffOption::Skip);
                }
            } else {
                // What's left of it, at the usual size.
                let mut chars = block.text.chars();
                let mut cx = x;
                while cx < area.right() && (cx == x || is_pad(buf, cx, y)) {
                    match chars.next() {
                        Some(c) => buf[(cx, y)].set_symbol(&c.to_string()),
                        None => buf[(cx, y)].set_symbol(" "),
                    };
                    cx += 1;
                }
            }
        }
    }
    // Rows below a block whose top scrolled away, and anything else left over.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if is_pad(buf, x, y) {
                buf[(x, y)].set_symbol(" ");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cursor_reports() {
        assert_eq!(columns(b"\x1b[12;1R\x1b[12;3R"), vec![1, 3]);
        assert_eq!(columns(b"\x1b[12;1R"), vec![1]);
    }

    #[test]
    fn large_text_fills_whole_blocks() {
        let chunks: Vec<(String, usize)> = large_chunks("Samuel").collect();
        assert_eq!(chunks, vec![("Samu".to_string(), 3), ("el".to_string(), 2)]);
    }

    /// Draws `lines` from (1, 1) through ratatui's crossterm backend, as the app does, and returns what's written.
    fn written(lines: &[ratatui::text::Line<'static>], height: u16) -> String {
        use ratatui::backend::CrosstermBackend;
        use ratatui::layout::Rect;
        use ratatui::{Terminal, TerminalOptions, Viewport};
        use std::rc::Rc;

        // The backend keeps its writer to itself, so it gets a handle on a shared one.
        #[derive(Clone, Default)]
        struct Shared(Rc<RefCell<Vec<u8>>>);
        impl std::io::Write for Shared {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.borrow_mut().write(bytes)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let _ = SUPPORTED.set(true);
        let out = Shared::default();
        let area = Rect::new(0, 0, 30, height);
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(out.clone()),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .unwrap();
        terminal
            .draw(|f| {
                for (i, line) in lines.iter().enumerate() {
                    f.buffer_mut().set_line(1, 1 + i as u16, line, 29);
                }
                finish(f.buffer_mut());
            })
            .unwrap();
        drop(terminal);
        String::from_utf8_lossy(&out.0.borrow()).into_owned()
    }

    #[test]
    fn intact_blocks_become_one_escape_and_the_row_below_is_left_to_the_terminal() {
        let _ = SUPPORTED.set(true);
        reset();
        let (top, bottom, w) = rows("Samuel", Size::Large, Style::default()).unwrap();
        assert_eq!(w, 10);
        let after = Span::raw(" @sam");
        let out = written(&[top.into(), ratatui::text::Line::from(vec![bottom, after])], 4);
        assert!(out.contains("\x1b]66;s=2:w=3:n=3:d=4:v=1;Samu\x07\x1b]66;s=2:w=2:n=3:d=4:v=1;el\x07"));
        // The handle is moved to on the second row, past the name's cells, which aren't written.
        assert!(out.contains("\x1b[3;13H@sam"), "{out:?}");
        assert!(!out.contains(PAD));
    }

    #[test]
    fn blocks_cut_off_go_back_to_plain_text() {
        let _ = SUPPORTED.set(true);
        reset();
        let (top, _, _) = rows("Samuel", Size::Large, Style::default()).unwrap();
        // The row below it is the screen's last, with something else in it.
        let out = written(&[top.into(), "something else".into()], 3);
        assert!(!out.contains("\x1b]66"));
        assert!(out.contains("Samuel"));
        assert!(!out.contains(PAD));
    }
}
