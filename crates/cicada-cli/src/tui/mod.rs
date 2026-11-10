//! `cicada tui`: a k9s-style terminal UI.
//!
//! [`app::App`] holds the state and turns input into effects; `fetch` talks to the API; `ui` draws. This module runs
//! the loop: it waits on input, a 250 ms tick and the results of spawned requests at once, so a slow node never
//! blocks the screen.

pub mod app;
mod fetch;
pub mod grid;
mod highlight;
mod theme;
mod ui;

use std::io::stdout;
use std::time::Duration;

use cicada_client::ActorClient;
use futures::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind};
use ratatui::crossterm::execute;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use self::app::{App, Effect, Fetched};
use crate::config::Target;

/// The UI tick: spinner, toasts, refresh schedule.
const TICK: Duration = Duration::from_millis(250);
/// How long one TUI request may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the TUI against `target` until the user quits. The terminal is restored on return and on panic.
pub async fn run(target: Target) -> anyhow::Result<()> {
    let client = target.client(TIMEOUT)?;
    let mut terminal = ratatui::init();
    let restore = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture);
        restore(info);
    }));
    execute!(stdout(), EnableMouseCapture)?;
    let result = event_loop(&mut terminal, client, App::new(target.context, target.server)).await;
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

async fn event_loop(terminal: &mut DefaultTerminal, client: ActorClient, mut app: App) -> anyhow::Result<()> {
    let (tx, mut results) = unbounded_channel();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(TICK);
    let mut effects = app.start();
    loop {
        for effect in effects.drain(..) {
            if effect == Effect::Quit {
                return Ok(());
            }
            spawn(&client, &tx, effect);
        }
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        effects = tokio::select! {
            event = events.next() => match event.transpose()? {
                Some(Event::Key(key)) if key.kind == KeyEventKind::Press => app.on_key(key),
                Some(Event::Mouse(mouse)) => app.on_mouse(mouse),
                Some(_) => vec![],
                None => return Ok(()),
            },
            _ = tick.tick() => app.on_tick(),
            Some(result) = results.recv() => app.on_result(result),
        };
    }
}

/// Runs `effect` in a task that sends its result back.
fn spawn(client: &ActorClient, tx: &UnboundedSender<Fetched>, effect: Effect) {
    let (client, tx) = (client.clone(), tx.clone());
    tokio::spawn(async move {
        let result = match effect {
            Effect::Fetch { seq, target } => match fetch::screen(&client, &target).await {
                Ok(data) => Fetched::Screen { seq, data },
                Err(e) => Fetched::Failed { seq: Some(seq), error: crate::error_text(&e) },
            },
            Effect::Summary => Fetched::Summary(fetch::summary(&client).await.map_err(|e| crate::error_text(&e))),
            Effect::Act(action) => match fetch::act(&client, &action).await {
                Ok(text) => Fetched::Done { action, text },
                Err(e) => Fetched::Failed { seq: None, error: crate::error_text(&e) },
            },
            Effect::Quit => return,
        };
        let _ = tx.send(result);
    });
}
