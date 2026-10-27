// Messages and loaded pages are made once per response, so a large variant costs nothing worth boxing for.
#![allow(clippy::large_enum_variant)]

mod activity;
mod api;
mod app;
mod attach;
mod auth;
mod callback;
mod cli_commit;
mod config;
mod gfx;
mod model;
mod remotes;
mod system;
mod text;
mod theme;
mod ui;

use std::io::{self, Stdout, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, EventStream, KeyEventKind,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::api::Api;
use crate::app::{App, CliJob, Msg};
use crate::auth::DeviceKey;
use crate::config::Config;

type Term = Terminal<CrosstermBackend<Stdout>>;

static KEYBOARD_ENHANCED: AtomicBool = AtomicBool::new(false);

const HELP: &str = "\
devnull — /dev/null in your terminal.

Usage:
  devnull                 Open the TUI
  devnull commit [--reply <commit>] [--image <path>]... <text>
                          Commit from the shell (signed in, or with your GitHub SSH key)
  devnull --help          Show this

Environment:
  DEVNULL_URL             The server, https://dev-null.sh by default
  DEVNULL_CONFIG_DIR      Where the config and device key live
";

#[tokio::main]
async fn main() {
    if let Err(e) = run_cli().await {
        eprintln!("devnull: {e}");
        std::process::exit(1);
    }
}

async fn run_cli() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("devnull {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // The OS runs this with the devnull:// link while signing in.
    if args.first().map(String::as_str) == Some("callback") {
        return callback::receive(&args[1..]);
    }

    let config = Config::load();
    let key = config
        .signed_in
        .then(|| DeviceKey::load(&config::key_path()))
        .flatten()
        .map(Arc::new);
    let api = Api::new(config::base_url(), key);

    if args.first().map(String::as_str) == Some("commit") {
        return commit_from_shell(api, &args[1..]).await;
    }

    gfx::init();
    let mut terminal = setup()?;
    // Asked even when the theme is set, so switching to auto later knows the answer.
    let detected = theme::detect_light();
    let result = run(&mut terminal, api, config, detected).await;
    restore(&mut terminal)?;
    result
}

/// `devnull commit [--reply <commit>] [--image <path>]... 'Shipped!'`, for scripts and muscle memory.
async fn commit_from_shell(api: Api, args: &[String]) -> Result<()> {
    let mut reply_to: Option<String> = None;
    let mut images = Vec::new();
    let mut rest = args;
    loop {
        match rest {
            [flag, value, tail @ ..] if flag == "--image" => {
                images.push(attach::Attachment::from_path(std::path::Path::new(value))?);
                rest = tail;
            }
            [flag, value, tail @ ..] if flag == "--reply" => {
                reply_to = Some(value.clone());
                rest = tail;
            }
            [flag, tail @ ..] if flag == "--" => {
                rest = tail;
                break;
            }
            _ => break,
        }
    }
    if images.len() > attach::MAX_IMAGES {
        anyhow::bail!("A commit can carry up to {} images.", attach::MAX_IMAGES);
    }
    let body = rest.join(" ").trim().to_string();
    if body.is_empty() && images.is_empty() {
        anyhow::bail!("Aborting commit due to empty commit message.");
    }
    // The signed API takes neither images nor a reply by hash, so those go through the GitHub SSH key.
    if api.is_signed() && images.is_empty() && reply_to.is_none() {
        let commit = api.create_commit(&body, None, None).await?;
        let pending = commit.status.as_deref() == Some("pending");
        println!(
            "Committed: {}{}",
            commit.html_url.unwrap_or_default(),
            if pending { " (pending review)" } else { "" }
        );
        return Ok(());
    }
    let key = cli_commit::find_key(&api).await?;
    let payload = cli_commit::payload(reply_to.as_deref(), &body, &images);
    let signature = cli_commit::sign(&key, &payload, !images.is_empty())?;
    println!("{}", cli_commit::send(&api, payload, signature, &images).await?);
    Ok(())
}

fn setup() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    // Lets terminals that can tell ⌃↵ from ↵ do so, for committing from the composer.
    // Asked once: asking again later would leave the terminal's answer on screen when raw mode is off.
    if supports_keyboard_enhancement().unwrap_or(false) {
        KEYBOARD_ENHANCED.store(true, Ordering::Relaxed);
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        gfx::cleanup();
        let _ = leave();
        default_hook(info);
    }));

    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

/// Clears the screen and has the next frame drawn whole. Not `Terminal::clear`, which asks the terminal where its
/// cursor is, and loses the answer to the event reader, giving up after two seconds. A failure here only costs a
/// stale frame, so it never ends the app.
fn redraw_from_scratch(terminal: &mut Term) {
    let _ = execute!(io::stdout(), crossterm::terminal::Clear(crossterm::terminal::ClearType::All));
    if let Ok(fresh) = Terminal::new(CrosstermBackend::new(io::stdout())) {
        *terminal = fresh;
    }
}

fn leave() -> Result<()> {
    let mut stdout = io::stdout();
    if KEYBOARD_ENHANCED.load(Ordering::Relaxed) {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    execute!(
        stdout,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    disable_raw_mode()?;
    Ok(())
}

fn restore(terminal: &mut Term) -> Result<()> {
    gfx::cleanup();
    leave()?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run(terminal: &mut Term, api: Api, config: Config, detected: Option<bool>) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(api, config, tx);
    app.detected_light = detected;
    app.apply_theme();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(120));

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        // Pictures the frame wanted, but the terminal doesn't have yet.
        for key in gfx::take_queue() {
            app.spawn(async move {
                let png = gfx::fetch(key.clone()).await;
                Msg::Picture(key, png)
            });
        }

        tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => app.on_key(key),
                Some(Ok(Event::Paste(text))) => app.on_paste(text),
                Some(Ok(Event::Mouse(mouse))) => app.on_mouse(mouse),
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            },
            Some(msg) = rx.recv() => app.on_msg(msg),
            _ = tick.tick() => app.on_tick(),
        }

        if let Some(job) = app.cli_job.take() {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            // crossterm's reader would race ssh-keygen for a passphrase typed on the terminal, so it stops meanwhile.
            drop(events);
            let result = cli_commit(terminal, &app, job).await;
            events = EventStream::new();
            app.on_cli_commit(result);
            redraw_from_scratch(terminal);
        }

        if app.quit {
            break;
        }
    }
    Ok(())
}

/// Commits through /api/cli/commits. When the key is a file rather than in the agent, ssh-keygen may ask for its
/// passphrase, so the TUI steps aside for it.
async fn cli_commit(terminal: &mut Term, app: &App, job: CliJob) -> Result<String> {
    let key = cli_commit::find_key(&app.api).await?;
    let payload = cli_commit::payload(job.reply_to.as_deref(), &job.body, &job.images);
    let with_images = !job.images.is_empty();

    let signature = if key.needs_terminal {
        leave()?;
        let mut stdout = io::stdout();
        writeln!(stdout, "\n  devnull: signing your commit with {}…\n", key.label)?;
        stdout.flush()?;
        let signature = cli_commit::sign(&key, &payload, with_images);
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
        if KEYBOARD_ENHANCED.load(Ordering::Relaxed) {
            let _ = execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        redraw_from_scratch(terminal);
        signature?
    } else {
        cli_commit::sign(&key, &payload, with_images)?
    };

    cli_commit::send(&app.api, payload, signature, &job.images).await
}
