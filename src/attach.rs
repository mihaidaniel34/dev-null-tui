//! Images attached to a commit: dropped or pasted into the terminal (which types their paths), or taken from the
//! clipboard. The limits are the server's: up to 3 JPEG, PNG or WebP images of up to 10 MB and 6000 pixels a side.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use image::ImageFormat;

pub const MAX_IMAGES: usize = 3;
const MAX_BYTES: usize = 10_240_000;
const MAX_SIDE: u32 = 6000;

#[derive(Clone)]
pub struct Attachment {
    pub name: String,
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

impl Attachment {
    pub fn from_bytes(name: String, bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            bail!("{name} is larger than 10 MB.");
        }
        let format = image::guess_format(&bytes).ok();
        let mime = match format {
            Some(ImageFormat::Jpeg) => "image/jpeg",
            Some(ImageFormat::Png) => "image/png",
            Some(ImageFormat::WebP) => "image/webp",
            _ => bail!("{name} isn't a JPEG, PNG or WebP image."),
        };
        let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format.expect("known"))
            .into_dimensions()
            .with_context(|| format!("Could not read {name}."))?;
        if width > MAX_SIDE || height > MAX_SIDE {
            bail!("{name} is {width}×{height}: images can be up to {MAX_SIDE} pixels a side.");
        }
        Ok(Self {
            name,
            bytes,
            mime,
            width,
            height,
        })
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "image".into());
        let size = fs::metadata(path).with_context(|| format!("Could not read {name}."))?.len();
        if size as usize > MAX_BYTES {
            bail!("{name} is larger than 10 MB.");
        }
        let bytes = fs::read(path).with_context(|| format!("Could not read {name}."))?;
        Self::from_bytes(name, bytes)
    }

    pub fn size_label(&self) -> String {
        let kb = self.bytes.len() as f64 / 1024.0;
        if kb < 1024.0 {
            format!("{kb:.0} KB")
        } else {
            format!("{:.1} MB", kb / 1024.0)
        }
    }

    pub fn sha256(&self) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(&self.bytes).iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// When everything pasted is paths of files that exist, as a terminal types them for dropped files, those files.
/// Handles the shell quoting terminals use (backslash-escaped spaces on macOS and Linux, quotes) and file:// URLs.
pub fn pasted_paths(text: &str) -> Option<Vec<PathBuf>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            // Windows paths are made of backslashes; elsewhere a backslash escapes the next character.
            (None, '\\') if !cfg!(windows) => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            (_, c) => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.is_empty() || tokens.len() > 10 {
        return None;
    }

    let home = dirs::home_dir();
    let paths: Vec<PathBuf> = tokens
        .iter()
        .map(|t| {
            if let Some(url) = t.strip_prefix("file://").and_then(|_| url::Url::parse(t).ok()) {
                return url.to_file_path().unwrap_or_default();
            }
            match (t.strip_prefix("~/"), &home) {
                (Some(rest), Some(home)) => home.join(rest),
                _ => PathBuf::from(t),
            }
        })
        .collect();

    paths.iter().all(|p| p.is_absolute() && p.is_file()).then_some(paths)
}

/// The image on the clipboard, such as a screenshot copied with ⌘⌃⇧4.
pub fn from_clipboard() -> Result<Attachment> {
    let bytes = clipboard_png()?;
    if bytes.is_empty() {
        bail!("There's no image on the clipboard.");
    }
    Attachment::from_bytes("clipboard.png".into(), bytes)
}

#[cfg(target_os = "macos")]
fn clipboard_png() -> Result<Vec<u8>> {
    let tmp = std::env::temp_dir().join(format!("devnull-clipboard-{}.png", crate::auth::random_hex(6)));
    let script = format!(
        "set f to open for access POSIX file \"{}\" with write permission\ntry\nwrite (the clipboard as «class PNGf») to f\nend try\nclose access f",
        tmp.display()
    );
    let ok = Command::new("osascript")
        .args(["-e", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();
    let bytes = fs::read(&tmp).unwrap_or_default();
    let _ = fs::remove_file(&tmp);
    if !ok {
        bail!("There's no image on the clipboard.");
    }
    Ok(bytes)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn clipboard_png() -> Result<Vec<u8>> {
    let attempts: [(&str, &[&str]); 2] = [
        ("wl-paste", &["--no-newline", "--type", "image/png"]),
        ("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"]),
    ];
    for (tool, args) in attempts {
        if let Ok(out) = Command::new(tool).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output()
            && out.status.success()
            && !out.stdout.is_empty()
        {
            return Ok(out.stdout);
        }
    }
    bail!("There's no image on the clipboard (reading it needs wl-paste or xclip).")
}

#[cfg(windows)]
fn clipboard_png() -> Result<Vec<u8>> {
    let tmp = std::env::temp_dir().join(format!("devnull-clipboard-{}.png", crate::auth::random_hex(6)));
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; $i = [Windows.Forms.Clipboard]::GetImage(); if ($i) {{ $i.Save('{}', [Drawing.Imaging.ImageFormat]::Png) }}",
        tmp.display()
    );
    Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    let bytes = fs::read(&tmp).unwrap_or_default();
    let _ = fs::remove_file(&tmp);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_dropped_paths() {
        let dir = std::env::temp_dir().join(format!("devnull attach {}", crate::auth::random_hex(4)));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("shot one.png");
        fs::write(&a, b"x").unwrap();
        // How the terminal types a dropped file: spaces escaped on macOS and Linux, the path quoted on Windows.
        let escaped = if cfg!(windows) { format!("\"{}\"", a.display()) } else { a.display().to_string().replace(' ', "\\ ") };
        assert_eq!(pasted_paths(&escaped), Some(vec![a.clone()]));
        assert_eq!(pasted_paths(&format!("'{}'", a.display())), Some(vec![a.clone()]));
        assert_eq!(pasted_paths(&format!("{} {}", escaped, escaped)).map(|p| p.len()), Some(2));
        assert_eq!(pasted_paths("just some text"), None);
        assert_eq!(pasted_paths(&format!("look at {escaped}")), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn it_checks_the_format() {
        let mut png = Vec::new();
        image::RgbImage::new(4, 3)
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let a = Attachment::from_bytes("a.png".into(), png).unwrap();
        assert_eq!((a.width, a.height, a.mime), (4, 3, "image/png"));
        assert!(Attachment::from_bytes("a.gif".into(), b"GIF89a".to_vec()).is_err());
    }
}
