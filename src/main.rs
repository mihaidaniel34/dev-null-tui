// Messages and loaded pages are made once per response, so a large variant costs nothing worth boxing for.
#![allow(clippy::large_enum_variant)]

mod api;
mod app;
mod auth;
mod cli_commit;
mod config;
mod model;
mod system;
mod text;
mod theme;
mod ui;

use std::io::{self, Stdout, Write};
use std::sync::Arc;
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
use crate::app::{App, CliJob};
use crate::auth::DeviceKey;
use crate::config::Config;

type Term = Terminal<CrosstermBackend<Stdout>>;

const HELP: &str = "\
devnull — /dev/null in your terminal.

Usage:
  devnull                 Open the TUI
  devnull commit <text>   Commit from the shell (signed in, or with your GitHub SSH key)
  devnull --help          Show this

Environment:
  DEVNULL_URL             The server, https://dev-null.sh by default
  DEVNULL_CONFIG_DIR      Where the config and device key live
";

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("devnull {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let config = Config::load();
    let key = config
        .signed_in
        .then(|| DeviceKey::load(&config::key_path()))
        .flatten()
        .map(Arc::new);
    let api = Api::new(config::base_url(), key);

    if args.first().map(String::as_str) == Some("commit") {
        return commit_from_shell(api, args[1..].join(" ")).await;
    }

    let mut terminal = setup()?;
    let result = run(&mut terminal, api, config).await;
    restore(&mut terminal)?;
    result
}

/// `devnull commit 'Shipped!'`, for scripts and muscle memory.
async fn commit_from_shell(api: Api, body: String) -> Result<()> {
    let body = body.trim().to_string();
    if body.is_empty() {
        anyhow::bail!("Aborting commit due to empty commit message.");
    }
    if api.is_signed() {
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
    let payload = cli_commit::payload(None, &body);
    let signature = cli_commit::sign(&key, &payload)?;
    println!("{}", cli_commit::send(&api, payload, signature).await?);
    Ok(())
}

fn setup() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    // Lets terminals that can tell ⌃↵ from ↵ do so, for committing from the composer.
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = leave();
        default_hook(info);
    }));

    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn leave() -> Result<()> {
    let mut stdout = io::stdout();
    if supports_keyboard_enhancement().unwrap_or(false) {
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
    leave()?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run(terminal: &mut Term, api: Api, config: Config) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(api, config, tx);
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(120));

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

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
            terminal.clear()?;
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
    let payload = cli_commit::payload(job.reply_to.as_deref(), &job.body);

    let signature = if key.needs_terminal {
        leave()?;
        let mut stdout = io::stdout();
        writeln!(stdout, "\n  devnull: signing your commit with {}…\n", key.label)?;
        stdout.flush()?;
        let signature = cli_commit::sign(&key, &payload);
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
        if supports_keyboard_enhancement().unwrap_or(false) {
            let _ = execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        terminal.clear()?;
        signature?
    } else {
        cli_commit::sign(&key, &payload)?
    };

    cli_commit::send(&app.api, payload, signature).await
}
