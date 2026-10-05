//! tetron-tui: interactive terminal UI for the tetron P2P mesh VPN.
//!
//! Slice A (read-only): an async event loop over a background IPC client that
//! polls the daemon's `Status` once a second, rendered as a btop-style
//! networks/peers/transport/history view. No actions change daemon state yet.

mod app;
mod ipc_client;
mod model;
mod ui;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, EventStream};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::time::Duration;
use tokio::sync::mpsc;

use app::App;
use model::ConnStatus;

type Term = Terminal<CrosstermBackend<Stdout>>;

#[tokio::main]
async fn main() -> Result<()> {
    let mut terminal = setup()?;
    let res = run(&mut terminal).await;
    teardown(&mut terminal)?;
    res
}

fn setup() -> Result<Term> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn teardown(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run(terminal: &mut Term) -> Result<()> {
    let mut app = App::new();
    let (tx, mut rx) = mpsc::channel::<ConnStatus>(8);
    tokio::spawn(ipc_client::run(tx));

    let mut events = EventStream::new();
    // A steady redraw cadence keeps relative times ("3m ago") fresh even when
    // nothing else changes.
    let mut tick = tokio::time::interval(Duration::from_millis(500));

    terminal.draw(|f| ui::draw(f, &mut app))?;
    loop {
        tokio::select! {
            status = rx.recv() => {
                if let Some(status) = status {
                    app.on_status(status);
                }
            }
            ev = events.next() => {
                if let Some(Ok(ev)) = ev {
                    app.on_event(ev);
                }
            }
            _ = tick.tick() => {}
        }
        if app.should_quit {
            break;
        }
        terminal.draw(|f| ui::draw(f, &mut app))?;
    }
    Ok(())
}
