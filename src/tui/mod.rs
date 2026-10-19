//! Terminal user interface built on ratatui.
//!
//! The UI thread owns the terminal and the `App` state; a worker thread owns
//! the `Workflow` and performs every git and model operation, reporting back
//! through a channel so the interface never blocks.

mod app;
#[cfg(test)]
mod tests;
mod ui;
mod widgets;
mod wizard;
mod worker;

use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::DefaultTerminal;

use crate::config::{Config, Confirmations};
use crate::update::spawn_update_check;
use crate::workflow::Workflow;
use crate::VERSION;

pub use app::{App, Mode};
pub use worker::{Cmd, Evt};

use app::Effect;

pub struct Options {
    pub instruction: Option<String>,
    pub model_label: String,
    pub debug: bool,
    pub confirmations: Confirmations,
}

const TICK: Duration = Duration::from_millis(60);

/// Runs the interactive commit workflow and returns the process exit code.
pub fn run_commit(workflow: Workflow, options: Options) -> Result<i32> {
    run_app(workflow, options, Mode::Commit)
}

/// Derives `.vibegitrules` from the history, lets the user review it, writes it.
pub fn run_init(workflow: Workflow, options: Options) -> Result<i32> {
    run_app(workflow, options, Mode::Init)
}

fn run_app(workflow: Workflow, options: Options, mode: Mode) -> Result<i32> {
    let repo_label = workflow.repo.root().display().to_string();
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (evt_tx, evt_rx) = mpsc::channel::<Evt>();
    std::thread::Builder::new()
        .name("vibegit-worker".into())
        .spawn(move || worker::run(workflow, cmd_rx, evt_tx))
        .context("could not start the worker thread")?;
    let update_rx = spawn_update_check(VERSION);

    let mut app = App::new(
        repo_label,
        options.model_label,
        options.instruction,
        options.debug,
        options.confirmations,
    );
    app.mode = mode;
    match mode {
        Mode::Commit => {
            cmd_tx.send(Cmd::Begin).ok();
        }
        Mode::Init => {
            app.status = "Analyzing the commit history...".into();
            cmd_tx.send(Cmd::DeriveRules).ok();
        }
    }

    let mut terminal = ratatui::init();
    let _ = execute!(io::stdout(), EnableMouseCapture);
    let result = event_loop(&mut terminal, &mut app, &cmd_tx, &evt_rx, &update_rx);
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    cmd_tx.send(Cmd::Quit).ok();
    let code = result?;
    for line in app.farewell() {
        println!("{line}");
    }
    Ok(code)
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    cmd_tx: &mpsc::Sender<Cmd>,
    evt_rx: &mpsc::Receiver<Evt>,
    update_rx: &mpsc::Receiver<Option<String>>,
) -> Result<i32> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(TICK)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Mouse(mouse) => app.on_mouse(mouse),
                _ => {}
            }
        }
        while let Ok(evt) = evt_rx.try_recv() {
            app.on_event(evt);
        }
        if let Ok(Some(latest)) = update_rx.try_recv() {
            app.update_notice = Some(latest);
        }
        if last_tick.elapsed() >= TICK {
            app.tick();
            last_tick = Instant::now();
        }

        for effect in app.take_effects() {
            match effect {
                Effect::Send(cmd) => {
                    cmd_tx.send(cmd).ok();
                }
                Effect::SaveConfirmations(confirmations) => {
                    if let Err(err) = save_confirmations(confirmations) {
                        app.status = format!("could not save the preference: {err:#}");
                    }
                }
                Effect::ExternalEdit { index, message } => {
                    let edited = suspend(terminal, || edit_in_external_editor(&message))?;
                    match edited {
                        Ok(text) => app.set_message(index, text),
                        Err(err) => app.status = format!("editor: {err:#}"),
                    }
                }
                Effect::ExternalEditRules(text) => {
                    let edited = suspend(terminal, || edit_in_external_editor(&text))?;
                    match edited {
                        Ok(text) => app.set_rules_text(text),
                        Err(err) => app.status = format!("editor: {err:#}"),
                    }
                }
            }
        }
        if app.should_quit {
            return Ok(app.exit_code);
        }
    }
}

/// Persists the "don't ask again" choice without touching other settings.
fn save_confirmations(confirmations: Confirmations) -> Result<()> {
    let (mut config, _) = Config::load()?;
    config.confirmations = confirmations;
    config.save()
}

/// Leaves the alternate screen and raw mode while `f` runs (editors, hooks).
fn suspend<T>(terminal: &mut DefaultTerminal, f: impl FnOnce() -> T) -> Result<T> {
    disable_raw_mode()?;
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    let result = f();
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    enable_raw_mode()?;
    terminal.clear()?;
    Ok(result)
}

fn edit_in_external_editor(message: &str) -> Result<String> {
    let path = std::env::temp_dir().join(format!("vibegit-message-{}.txt", std::process::id()));
    std::fs::write(&path, format!("{}\n", message.trim_end()))?;
    let outcome = crate::cli::open_in_editor(&path);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    outcome?;
    Ok(text.trim_end().to_string())
}

/// Runs the configuration wizard; returns the saved configuration, or `None` when cancelled.
pub fn run_wizard(config: Config) -> Result<Option<Config>> {
    let mut terminal = ratatui::init();
    let result = wizard::run(&mut terminal, config);
    ratatui::restore();
    result
}
