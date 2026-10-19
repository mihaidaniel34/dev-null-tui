//! Catching the `devnull://auth/callback` link the authorize page redirects to.
//!
//! That link is meant for the mobile app, so on a computer the browser drops it. While signing in, devnull registers
//! itself as the handler for `devnull://` (a small AppleScript app on macOS, a .desktop entry on Linux, a registry key
//! on Windows), which runs `devnull callback <link>`. That writes the link to a file the TUI is watching, and the
//! handler is removed again once signing in is over.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const SCHEME: &str = "devnull";

fn file(dir: &Path) -> PathBuf {
    dir.join("callback")
}

/// `devnull callback [--config-dir <dir>] <link>`, which the OS runs with the link.
pub fn receive(args: &[String]) -> Result<()> {
    let (dir, link) = match args {
        [flag, dir, link] if flag == "--config-dir" => (PathBuf::from(dir), link),
        [link] => (config::dir(), link),
        _ => bail!("Usage: devnull callback [--config-dir <dir>] <devnull://auth/callback?...>"),
    };
    if !link.starts_with("devnull://") || link.len() > 2048 {
        bail!("That isn't a devnull:// link.");
    }
    fs::create_dir_all(&dir)?;
    let tmp = dir.join("callback.tmp");
    write_private(&tmp, link.as_bytes())?;
    fs::rename(&tmp, file(&dir)).context("Could not hand the link to devnull.")
}

/// The link the browser handed over, if it has.
pub fn take() -> Option<String> {
    let path = file(&config::dir());
    let link = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);
    Some(link.trim().to_string()).filter(|l| !l.is_empty())
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let _ = fs::remove_file(path);
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    Ok(fs::write(path, bytes)?)
}

fn quiet(command: &mut Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Registered while it's alive, and removed when dropped.
pub struct Handler {
    cleanup: Cleanup,
}

enum Cleanup {
    #[cfg(target_os = "macos")]
    MacApp(PathBuf),
    #[cfg(target_os = "linux")]
    Desktop { file: PathBuf, previous: Option<String> },
    #[cfg(windows)]
    Registry,
    #[allow(dead_code)]
    Nothing,
}

impl Drop for Handler {
    fn drop(&mut self) {
        match &self.cleanup {
            #[cfg(target_os = "macos")]
            Cleanup::MacApp(app) => {
                quiet(Command::new(LSREGISTER).arg("-u").arg(app));
                let _ = fs::remove_dir_all(app);
            }
            #[cfg(target_os = "linux")]
            Cleanup::Desktop { file, previous } => {
                let _ = fs::remove_file(file);
                if let Some(previous) = previous {
                    quiet(Command::new("xdg-mime").args(["default", previous, &format!("x-scheme-handler/{SCHEME}")]));
                }
            }
            #[cfg(windows)]
            Cleanup::Registry => {
                quiet(Command::new("reg").args(["delete", r"HKCU\Software\Classes\devnull", "/f"]));
            }
            Cleanup::Nothing => {}
        }
    }
}

/// Registers devnull as the `devnull://` handler for this sign-in. None when this system can't, and the link has
/// to be pasted instead.
pub fn install() -> Option<Handler> {
    let _ = fs::remove_file(file(&config::dir()));
    let exe = std::env::current_exe().ok()?;
    platform_install(&exe, &config::dir()).ok()
}

#[cfg(target_os = "macos")]
const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

#[cfg(target_os = "macos")]
fn platform_install(exe: &Path, dir: &Path) -> Result<Handler> {
    // macOS hands URLs to apps as an Apple Event rather than an argument, so a two-line AppleScript app receives it
    // and passes it on to `devnull callback`.
    let app = dir.join("devnull sign-in.app");
    let _ = fs::remove_dir_all(&app);
    fs::create_dir_all(dir)?;

    let literal = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let command = format!(
        "quoted form of {} & \" callback --config-dir \" & quoted form of {} & \" \" & quoted form of theURL",
        literal(&exe.display().to_string()),
        literal(&dir.display().to_string()),
    );
    let compiled = Command::new("osacompile")
        .arg("-o")
        .arg(&app)
        .args([
            "-e",
            "on open location theURL",
            "-e",
            &format!("do shell script {command}"),
            "-e",
            "end open location",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !compiled.success() {
        bail!("osacompile failed");
    }

    let plist = app.join("Contents/Info.plist");
    for edit in [
        "Set :CFBundleIdentifier sh.dev-null.tui.sign-in",
        "Add :LSUIElement bool true",
        "Add :CFBundleURLTypes array",
        "Add :CFBundleURLTypes:0 dict",
        "Add :CFBundleURLTypes:0:CFBundleURLName string devnull sign-in",
        "Add :CFBundleURLTypes:0:CFBundleURLSchemes array",
        "Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string devnull",
    ] {
        quiet(Command::new("/usr/libexec/PlistBuddy").args(["-c", edit]).arg(&plist));
    }
    // The edit breaks the app's ad hoc signature, so it's signed again.
    quiet(Command::new("codesign").args(["--force", "--deep", "--sign", "-"]).arg(&app));

    let handler = Handler {
        cleanup: Cleanup::MacApp(app.clone()),
    };
    if !quiet(Command::new(LSREGISTER).arg("-f").arg(&app)) {
        bail!("lsregister failed");
    }
    // Wins over the /dev/null app itself, if it's installed on this Mac, until the handler is removed.
    quiet(Command::new("osascript").args([
        "-l",
        "JavaScript",
        "-e",
        "ObjC.import('CoreServices'); $.LSSetDefaultHandlerForURLScheme($('devnull'), $('sh.dev-null.tui.sign-in'))",
    ]));
    Ok(handler)
}

#[cfg(target_os = "linux")]
fn platform_install(exe: &Path, dir: &Path) -> Result<Handler> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        bail!("no desktop");
    }
    let mime = format!("x-scheme-handler/{SCHEME}");
    let previous = Command::new("xdg-mime")
        .args(["query", "default", &mime])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|p| !p.is_empty() && p != "devnull-tui-sign-in.desktop");

    // Desktop entry Exec quoting: double quotes, with " ` $ and \ escaped.
    let quote = |s: &str| {
        let escaped: String = s
            .chars()
            .flat_map(|c| {
                if matches!(c, '"' | '`' | '$' | '\\') {
                    vec!['\\', c]
                } else {
                    vec![c]
                }
            })
            .collect();
        format!("\"{escaped}\"")
    };
    let applications = dirs::data_dir().context("no data dir")?.join("applications");
    fs::create_dir_all(&applications)?;
    let file = applications.join("devnull-tui-sign-in.desktop");
    fs::write(
        &file,
        format!(
            "[Desktop Entry]\nType=Application\nName=devnull sign-in\nExec={} callback --config-dir {} %u\nNoDisplay=true\nMimeType={mime};\n",
            quote(&exe.display().to_string()),
            quote(&dir.display().to_string()),
        ),
    )?;
    let handler = Handler {
        cleanup: Cleanup::Desktop { file, previous },
    };
    if !quiet(Command::new("xdg-mime").args(["default", "devnull-tui-sign-in.desktop", &mime])) {
        bail!("xdg-mime failed");
    }
    quiet(Command::new("update-desktop-database").arg(&applications));
    Ok(handler)
}

#[cfg(windows)]
fn platform_install(exe: &Path, dir: &Path) -> Result<Handler> {
    let key = r"HKCU\Software\Classes\devnull";
    // Something else owns devnull:// here, so it's left alone.
    if quiet(Command::new("reg").args(["query", key])) {
        bail!("devnull:// is taken");
    }
    let command = format!("\"{}\" callback --config-dir \"{}\" \"%1\"", exe.display(), dir.display());
    let handler = Handler {
        cleanup: Cleanup::Registry,
    };
    let ok = quiet(Command::new("reg").args(["add", key, "/ve", "/d", "URL:devnull", "/f"]))
        && quiet(Command::new("reg").args(["add", key, "/v", "URL Protocol", "/d", "", "/f"]))
        && quiet(Command::new("reg").args(["add", &format!(r"{key}\shell\open\command"), "/ve", "/d", &command, "/f"]));
    if !ok {
        bail!("reg failed");
    }
    Ok(handler)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_install(_exe: &Path, _dir: &Path) -> Result<Handler> {
    bail!("unsupported")
}
