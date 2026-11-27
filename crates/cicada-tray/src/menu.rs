use std::cell::RefCell;
use std::process::ExitCode;
use std::time::Duration;

use chrono::FixedOffset;
use cicada::store::{Delivery, TurnOutcome};
use cicada::time::{Timestamp, add, between};
use tokio::time::Instant;

use crate::{FinishedTurn, RunningTurn, TrayState, TurnResult, TurnSnapshot};

/// The header of the Registered Types submenu.
pub(crate) const ACTOR_TYPES: &str = "Actor Types";
/// The second section of the Registered Types submenu, with its only row.
pub(crate) const BACKGROUND_TASKS: (&str, &str) = ("Background Tasks", "Not available over HTTP");
/// The last row of the Registered Types submenu, which opens Settings.
pub(crate) const CHOOSE_TYPES: &str = "Choose Types…";
/// The header and the footer of the Recent Turns submenu.
pub(crate) const RECENT: (&str, &str) = ("Finished on this worker", "Kept until the app quits");
/// The help under Stop Now.
pub(crate) const STOP_NOW_HELP: &str = "Stopping now cancels these turns. Their messages stay and run again later.";
/// The longest failure text the menu shows.
pub(crate) const FAILURE_LENGTH: usize = 200;
/// The longest turn error a recent row shows.
pub(crate) const TURN_ERROR_LENGTH: usize = 80;

pub(crate) struct MenuInput<'a> {
    /// The app's name: the header says "<name> worker", the last item "Quit <name>".
    pub title: &'a str,
    pub state: &'a TrayState,
    pub turns: &'a TurnSnapshot,
    pub registered_types: &'a [&'static str],
    /// The connection's worker, once it was ready.
    pub worker: Option<Worker<'a>>,
    /// When the stop of the connection began, and whether Stop Now was used.
    pub stopping: Option<(Instant, bool)>,
    /// When the last connection failed, by the wall clock.
    pub failed_at: Option<Timestamp>,
    /// Each context's name and its server's host.
    pub contexts: &'a [(String, Option<String>)],
    /// The context Connect targets while not connected.
    pub selected: Option<&'a str>,
    pub now: Timestamp,
    /// The local time zone's offset, for the times of day shown.
    pub zone: FixedOffset,
    pub monotonic_now: Instant,
    pub quitting: bool,
}

/// What the menu needs of a ready worker.
#[derive(Clone, Copy)]
pub(crate) struct Worker<'a> {
    pub types: &'a [&'static str],
    pub concurrency: usize,
    pub grace_period: Duration,
}

#[derive(PartialEq)]
pub(crate) struct MenuModel {
    pub header: Header,
    /// The grace period's bar in the header, while stopping a worker that was ready.
    pub bar: Option<Bar>,
    /// The status item's hexagon is filled: only while online.
    pub connected: bool,
    /// The last connection failure, while not connected.
    pub failure: Option<Row>,
    /// "Running" while connected, "Finishing" while disconnecting.
    pub running: Option<Section>,
    /// Hidden while disconnecting.
    pub contexts: Option<Vec<ContextRow>>,
    pub contexts_enabled: bool,
    /// Registered Types and Recent Turns, while connecting or connected.
    pub submenus: Option<Submenus>,
    /// "Connect to <context>", while not connected with a selected context.
    pub connect: Option<String>,
    /// Offer Stop Now and its help: while stopping a worker that was ready.
    pub stop_now: bool,
    pub inspector: Option<InspectorItem>,
    pub settings: bool,
    pub quit: String,
    pub quit_enabled: bool,
}

/// The inspector menu item, when the feature is built in.
#[derive(PartialEq, Eq)]
pub(crate) struct InspectorItem {
    pub title: &'static str,
    pub key: &'static str,
    pub enabled: bool,
}

impl InspectorItem {
    /// Creates the item in feature-enabled builds. A quit in progress disables it.
    pub fn entry(quitting: bool) -> Option<Self> {
        cfg!(feature = "inspector").then_some(Self { title: "Open Inspector…", key: "i", enabled: !quitting })
    }

    /// Opens on the connected context, or lets the inspector resolve its own target when offline.
    #[cfg(all(feature = "inspector", any(target_os = "macos", test)))]
    pub fn show(control: &RefCell<MenuControl>, state: &TrayState, show: impl FnOnce(Option<&str>)) {
        let enabled = !control.borrow().quitting();
        if enabled {
            show(state.connection());
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub title: String,
    pub subtitle: String,
    pub on: bool,
    pub enabled: bool,
}

/// The disconnecting bar: its fill from 0 to 1, the text on the left and the seconds on the right.
#[derive(Debug, PartialEq)]
pub(crate) struct Bar {
    pub fraction: f64,
    pub text: &'static str,
    pub left: Option<String>,
    pub label: String,
}

/// The icon of a rich row: the board's glyph in a tinted circle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Icon {
    /// Green pulse.
    Running,
    /// Orange clock.
    Finishing,
    /// Server glyph, blue filled when selected, grey otherwise.
    Context { selected: bool },
    /// Green check.
    Done,
    /// Red cross: failed, rescheduled and abandoned turns.
    Failed,
    /// Grey cross.
    Cancelled,
    /// Red exclamation mark: the connection failure.
    Alert,
}

/// A rich row: an icon, a title, a second line and the text on the right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub icon: Icon,
    pub title: String,
    pub subtitle: String,
    pub right: Option<String>,
}

/// A section header with its text on the right, and its rows.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Section {
    pub title: &'static str,
    pub right: String,
    pub rows: Vec<RunningRow>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RunningRow {
    pub turn: RunningTurn,
    pub row: Row,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContextRow {
    pub name: String,
    pub row: Row,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Submenus {
    /// "K of T": the registered types the worker runs, once it is ready.
    pub registered_right: Option<String>,
    pub types: Vec<TypeRow>,
    /// "N failed", when any recent turn failed.
    pub recent_right: Option<String>,
    pub recent: Vec<RecentRow>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TypeRow {
    pub name: &'static str,
    /// The worker runs it: a checkmark.
    pub runs: bool,
    /// "N running" or "Not on this worker", once the worker is ready.
    pub right: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RecentRow {
    pub turn: FinishedTurn,
    pub row: Row,
}

/// The title and subtitle of a row: on macOS 14.0 to 14.3, which have no subtitles, the second line joins the title.
pub(crate) fn row_text(row: &Row, subtitles: bool) -> (String, Option<String>) {
    if subtitles || row.subtitle.is_empty() {
        (row.title.clone(), (!row.subtitle.is_empty()).then(|| row.subtitle.clone()))
    } else {
        (format!("{} · {}", row.title, row.subtitle), None)
    }
}

/// Checks the minimum macOS version and reports whether it supports menu item subtitles.
pub(crate) fn macos_subtitles(version: (isize, isize, isize)) -> Result<bool, (&'static str, ExitCode)> {
    if version < (14, 0, 0) {
        return Err(("the Cicada menu bar app requires macOS 14 or later", ExitCode::FAILURE));
    }
    Ok(version >= (14, 4, 0))
}

impl TrayState {
    /// The context with a connection, including one being started or stopped.
    pub(crate) fn connection(&self) -> Option<&str> {
        match self {
            Self::Offline | Self::Failed { .. } => None,
            Self::Connecting { context } | Self::Online { context } | Self::Stopping { context, .. } => Some(context),
        }
    }
}

/// `text` as the menu shows it: the user, password, query and fragment of each URL removed, each run of control
/// characters and white space made one space, and cut at `limit` characters, the last an ellipsis when cut. The URLs
/// are cleaned before the cut, so a cut never leaves part of a credential.
pub(crate) fn display_text(text: &str, limit: usize) -> String {
    let mut shown = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        shown.push_str(&rest[..at + 3]);
        let after = &rest[at + 3..];
        let end = url_end(after);
        let url = &after[..end];
        // The user and password end at the last `@` before the path, also when they hold `?`, `#` or parentheses.
        let host = url[..url.find('/').unwrap_or(url.len())].rfind('@').map_or(0, |at| at + 1);
        let kept = url[host..].find(['?', '#']).map_or(url.len(), |query| host + query);
        shown.push_str(&url[host..kept]);
        rest = &after[end..];
    }
    shown.push_str(rest);
    let words = shown.split(|c: char| c.is_whitespace() || c.is_control()).filter(|word| !word.is_empty());
    let joined = words.collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let mut cut: String = joined.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Where a URL that starts `text` ends: at white space, a control character, a quote or an angle bracket, or at a `)`
/// that closes a parenthesis opened before the URL. Parentheses opened inside it belong to it.
fn url_end(text: &str) -> usize {
    let mut depth = 0usize;
    for (at, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return at,
            ')' => depth -= 1,
            c if c.is_whitespace() || c.is_control() || "<>\"'`".contains(c) => return at,
            _ => {}
        }
    }
    text.len()
}

fn clock(at: Timestamp, zone: FixedOffset) -> String {
    at.with_timezone(&zone).format("%H:%M").to_string()
}

/// A duration as m:ss.
fn minutes(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn running_row(turn: &RunningTurn, icon: Icon, now: Timestamp) -> RunningRow {
    let row = Row {
        icon,
        title: turn.actor.to_string(),
        subtitle: format!("{} · attempt {}", turn.message_type, turn.attempt),
        right: Some(minutes(between(turn.claimed_at, now))),
    };
    RunningRow { turn: turn.clone(), row }
}

/// A finished turn's row, and whether it counts as failed.
fn recent_row(turn: &FinishedTurn, zone: FixedOffset) -> (RecentRow, bool) {
    let kind = &turn.turn.message_type;
    let error = |error: &str| display_text(error.lines().next().unwrap_or_default(), TURN_ERROR_LENGTH);
    let (icon, line, failed) = match &turn.outcome {
        TurnResult::Committed(TurnOutcome::Done) => {
            (Icon::Done, format!("done in {}", minutes(between(turn.turn.claimed_at, turn.finished_at))), false)
        }
        TurnResult::Committed(TurnOutcome::Failed(reason)) => (Icon::Failed, error(reason), true),
        TurnResult::Committed(TurnOutcome::Rescheduled { delivery, error: reason }) => {
            let retry = match delivery {
                Delivery::Now => "now".to_owned(),
                Delivery::At(at) => clock(*at, zone),
                Delivery::After(delay) => clock(add(turn.finished_at, *delay).unwrap_or(turn.finished_at), zone),
            };
            (Icon::Failed, format!("{}, retry {retry}", error(reason)), true)
        }
        TurnResult::Committed(TurnOutcome::Cancelled) => (Icon::Cancelled, "cancelled".to_owned(), false),
        TurnResult::Abandoned => (Icon::Failed, "abandoned".to_owned(), true),
    };
    let row = Row {
        icon,
        title: turn.turn.actor.to_string(),
        subtitle: format!("{kind} · {line}"),
        right: Some(clock(turn.finished_at, zone)),
    };
    (RecentRow { turn: turn.clone(), row }, failed)
}

/// The disconnecting bar, from when the stop began to `until`, the tray's deadline: the grace period plus the cancel
/// timeout. The worker's grace period starts only once its polling loop stopped, which the tray does not see, so the
/// bar runs to that deadline. It is full, cancelling, after Stop Now, with no grace period, or past the deadline.
fn bar(grace: Duration, began: Instant, until: Instant, stopped_now: bool, now: Instant) -> Bar {
    if stopped_now || grace.is_zero() || now >= until {
        let label = "Disconnecting: cancelling turns".to_owned();
        return Bar { fraction: 1.0, text: "Cancelling turns…", left: None, label };
    }
    let total = until.saturating_duration_since(began);
    let elapsed = now.saturating_duration_since(began);
    let left = until - now;
    let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    let label = format!("Disconnecting: {} of {} seconds used", elapsed.as_secs(), total.as_secs());
    let fraction = elapsed.as_secs_f64() / total.as_secs_f64();
    Bar { fraction, text: "Taking no new turns", left: Some(format!("{seconds} s left")), label }
}

pub(crate) fn menu(input: &MenuInput<'_>) -> MenuModel {
    let selected = input.selected.filter(|name| input.contexts.iter().any(|(context, _)| context == name));
    let marked = input.state.connection().or(selected);
    let enabled = !input.quitting;
    let header = |subtitle: String, on: bool, switch: bool| Header {
        title: format!("{} worker", input.title),
        subtitle,
        on,
        enabled: switch && enabled,
    };
    let contexts = || {
        input
            .contexts
            .iter()
            .map(|(name, host)| ContextRow {
                name: name.clone(),
                row: Row {
                    icon: Icon::Context { selected: Some(name.as_str()) == marked },
                    title: name.clone(),
                    subtitle: host.clone().unwrap_or_default(),
                    right: None,
                },
            })
            .collect::<Vec<_>>()
    };
    let rows = |icon| input.turns.running.iter().map(|turn| running_row(turn, icon, input.now)).collect::<Vec<_>>();
    let mut model = MenuModel {
        header: header(String::new(), false, false),
        bar: None,
        connected: matches!(input.state, TrayState::Online { .. }),
        failure: None,
        running: None,
        contexts: None,
        contexts_enabled: enabled,
        submenus: None,
        connect: None,
        stop_now: false,
        inspector: InspectorItem::entry(input.quitting),
        settings: true,
        quit: format!("Quit {}", input.title),
        quit_enabled: enabled,
    };
    match input.state {
        TrayState::Offline | TrayState::Failed { .. } => {
            let subtitle =
                if input.contexts.is_empty() { "Not connected · Run cicada config" } else { "Not connected" };
            model.header = header(subtitle.to_owned(), false, selected.is_some());
            if let TrayState::Failed { context, error } = input.state {
                let error = display_text(error, FAILURE_LENGTH);
                let subtitle = match input.failed_at {
                    Some(at) => format!("{error} at {}", clock(at, input.zone)),
                    None => error,
                };
                let title = format!("Couldn't reach {context}");
                model.failure = Some(Row { icon: Icon::Alert, title, subtitle, right: None });
            }
            // Turns a stop left behind stay listed until they end (requirement "Turns after Stop Now").
            let rows = rows(Icon::Finishing);
            if !rows.is_empty() {
                model.running = Some(Section { title: "Finishing", right: rows.len().to_string(), rows });
            }
            model.contexts = Some(contexts());
            model.connect = selected.map(|name| format!("Connect to {name}"));
        }
        TrayState::Connecting { context } => {
            model.header = header(format!("Connecting to {context}…"), true, true);
            model.contexts = Some(contexts());
            model.submenus = Some(submenus(input));
        }
        TrayState::Online { context } => {
            model.header = header(format!("Connected to {context}"), true, true);
            let rows = rows(Icon::Running);
            let limit = input.worker.map_or(0, |worker| worker.concurrency);
            model.running = Some(Section { title: "Running", right: format!("{} of {limit}", rows.len()), rows });
            model.contexts = Some(contexts());
            model.submenus = Some(submenus(input));
        }
        TrayState::Stopping { context, until } => {
            model.header = header(format!("Disconnecting from {context}…"), false, false);
            if let (Some(worker), Some((began, stopped_now))) = (input.worker, input.stopping) {
                model.bar = Some(bar(worker.grace_period, began, *until, stopped_now, input.monotonic_now));
                model.stop_now = true;
            }
            let rows = rows(Icon::Finishing);
            model.running = Some(Section { title: "Finishing", right: rows.len().to_string(), rows });
            model.inspector = None;
            model.settings = false;
        }
    }
    model
}

fn submenus(input: &MenuInput<'_>) -> Submenus {
    let worker_types = input.worker.map_or(&[][..], |worker| worker.types);
    let types: Vec<_> = input
        .registered_types
        .iter()
        .map(|&name| {
            let runs = worker_types.contains(&name);
            let running = input.turns.running.iter().filter(|turn| turn.actor.type_name().as_str() == name).count();
            let right =
                input.worker.map(|_| if runs { format!("{running} running") } else { "Not on this worker".to_owned() });
            TypeRow { name, runs, right }
        })
        .collect();
    let runs = types.iter().filter(|row| row.runs).count();
    let mut failures = 0;
    let recent = input
        .turns
        .finished
        .iter()
        .rev()
        .map(|turn| {
            let (row, failed) = recent_row(turn, input.zone);
            failures += usize::from(failed);
            row
        })
        .collect();
    Submenus {
        registered_right: input.worker.map(|_| format!("{runs} of {}", types.len())),
        types,
        recent_right: (failures > 0).then(|| format!("{failures} failed")),
        recent,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Connect(String),
    Disconnect,
    StopApp,
    ReplyToTerminate,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Terminate {
    Now,
    Later(Option<Action>),
}

/// How the tray is asked to quit.
#[derive(Clone, Copy)]
pub(crate) enum QuitRequest {
    /// The Quit item or Ctrl-C.
    Local,
    /// AppKit, including system logout.
    System,
}

impl QuitRequest {
    /// Marks the tray as quitting and closes its inspector before dispatching the decision.
    pub fn begin(self, control: &RefCell<MenuControl>, state: &TrayState, close_inspector: impl FnOnce()) -> Terminate {
        let decision = match self {
            Self::Local => Terminate::Later(control.borrow_mut().quit(state)),
            Self::System => control.borrow_mut().terminate(state),
        };
        close_inspector();
        decision
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quit {
    App,
    System,
    Finishing,
}

#[derive(Default)]
pub(crate) struct MenuControl {
    pending: Option<String>,
    /// The last context picked; never written to the config file.
    picked: Option<String>,
    quit: Option<Quit>,
}

impl MenuControl {
    pub fn quitting(&self) -> bool {
        self.quit.is_some()
    }

    /// The context Connect and the switch use: the last pick while it is still configured, else the config file's
    /// current context while it is configured.
    pub fn selected<'a>(&'a self, names: &[String], current: Option<&'a str>) -> Option<&'a str> {
        let known = |name: &&str| names.iter().any(|known| known == name);
        self.picked.as_deref().filter(known).or(current.filter(known))
    }

    pub fn toggle(&self, state: &TrayState, on: bool, current: Option<&str>) -> Option<Action> {
        if self.quitting() {
            return None;
        }
        match (on, state) {
            (true, TrayState::Offline | TrayState::Failed { .. }) => current.map(|name| Action::Connect(name.into())),
            (false, TrayState::Connecting { .. } | TrayState::Online { .. }) => Some(Action::Disconnect),
            _ => None,
        }
    }

    /// Picks a context: while not connected, selects it without connecting; while connected to another, switches.
    pub fn pick(&mut self, state: &TrayState, name: String) -> Option<Action> {
        if self.quitting() {
            return None;
        }
        self.picked = Some(name.clone());
        match state {
            TrayState::Offline | TrayState::Failed { .. } => {
                self.pending = None;
                None
            }
            TrayState::Stopping { .. } => {
                self.pending = Some(name);
                None
            }
            TrayState::Online { context } | TrayState::Connecting { context } if *context != name => {
                self.pending = Some(name);
                Some(Action::Disconnect)
            }
            _ => None,
        }
    }

    /// Apply and Reconnect: while the tray holds a connection to any context, disconnects (unless already stopping)
    /// and connects to `name` once there is none, replacing a pending pick; with no connection only clears a pending
    /// pick; during a quit does nothing.
    pub fn apply(&mut self, state: &TrayState, name: String) -> Option<Action> {
        if self.quitting() {
            return None;
        }
        match state {
            TrayState::Offline | TrayState::Failed { .. } => {
                self.pending = None;
                None
            }
            TrayState::Stopping { .. } => {
                self.pending = Some(name);
                None
            }
            TrayState::Online { .. } | TrayState::Connecting { .. } => {
                self.pending = Some(name);
                Some(Action::Disconnect)
            }
        }
    }

    pub fn quit(&mut self, state: &TrayState) -> Option<Action> {
        if self.quitting() {
            return None;
        }
        self.begin_quit(state, Quit::App)
    }

    pub fn terminate(&mut self, state: &TrayState) -> Terminate {
        if self.quit == Some(Quit::System) {
            return Terminate::Later(None);
        }
        if state.connection().is_none() {
            self.begin_quit(state, Quit::Finishing);
            return Terminate::Now;
        }
        Terminate::Later(self.begin_quit(state, Quit::System))
    }

    fn begin_quit(&mut self, state: &TrayState, quit: Quit) -> Option<Action> {
        let was_quitting = self.quitting();
        self.quit = Some(quit);
        self.pending = None;
        match state {
            TrayState::Online { .. } | TrayState::Connecting { .. } if !was_quitting => Some(Action::Disconnect),
            _ => None,
        }
    }

    pub fn tick(&mut self, state: &TrayState) -> Option<Action> {
        if state.connection().is_some() {
            return None;
        }
        match self.quit {
            Some(Quit::App) => {
                self.quit = Some(Quit::Finishing);
                Some(Action::StopApp)
            }
            Some(Quit::System) => {
                self.quit = Some(Quit::Finishing);
                Some(Action::ReplyToTerminate)
            }
            Some(Quit::Finishing) => None,
            None => {
                let name = self.pending.take()?;
                self.picked = Some(name.clone());
                Some(Action::Connect(name))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cicada::time::from_micros;

    use super::*;

    const SECOND: i64 = 1_000_000;

    /// The menu's inputs; `model` turns them into the menu. Times are UTC and the clock reads 14:32:00.
    struct Fixture {
        state: TrayState,
        turns: TurnSnapshot,
        ready: bool,
        stopping: Option<(Instant, bool)>,
        grace: Duration,
        failed_at: Option<Timestamp>,
        contexts: Vec<(String, Option<String>)>,
        selected: Option<&'static str>,
        monotonic_now: Instant,
        quitting: bool,
    }

    /// A time of day, UTC, on 2026-11-18.
    fn at(hour: i64, minute: i64, second: i64) -> Timestamp {
        from_micros((1_794_960_000 + hour * 3600 + minute * 60 + second) * SECOND)
    }

    impl Fixture {
        fn new(state: TrayState) -> Self {
            Self {
                state,
                turns: TurnSnapshot::default(),
                ready: true,
                stopping: None,
                grace: Duration::from_secs(30),
                failed_at: None,
                contexts: vec![
                    ("local".into(), Some("127.0.0.1:8080".into())),
                    ("prod".into(), Some("actors.internal".into())),
                    ("staging".into(), Some("staging.actors.internal".into())),
                ],
                selected: Some("prod"),
                monotonic_now: Instant::now(),
                quitting: false,
            }
        }

        fn model(&self) -> MenuModel {
            menu(&MenuInput {
                title: "myapp",
                state: &self.state,
                turns: &self.turns,
                registered_types: &["reports", "invoices", "mailer", "audit"],
                worker: self.ready.then_some(Worker {
                    types: &["reports", "invoices", "mailer"],
                    concurrency: 4,
                    grace_period: self.grace,
                }),
                stopping: self.stopping,
                failed_at: self.failed_at,
                contexts: &self.contexts,
                selected: self.selected,
                now: at(14, 32, 0),
                zone: FixedOffset::east_opt(0).unwrap(),
                monotonic_now: self.monotonic_now,
                quitting: self.quitting,
            })
        }
    }

    fn online() -> TrayState {
        TrayState::Online { context: "prod".into() }
    }

    fn failed() -> TrayState {
        TrayState::Failed { context: "staging".into(), error: "Connection refused".into() }
    }

    fn stopping(until: Instant) -> TrayState {
        TrayState::Stopping { context: "prod".into(), until }
    }

    fn running(actor: &str, kind: &str, attempt: u32, claimed_at: Timestamp) -> RunningTurn {
        RunningTurn { actor: actor.parse().unwrap(), message_type: kind.into(), attempt, claimed_at }
    }

    /// The running turns of board Main: claimed 0:42, 0:07 and 0:03 ago.
    fn main_turns() -> Vec<RunningTurn> {
        vec![
            running("reports/2026-q3", "GenerateReport", 1, at(14, 31, 18)),
            running("invoices/inv-10422", "Render", 2, at(14, 31, 53)),
            running("mailer/daily-digest", "Send", 1, at(14, 31, 57)),
        ]
    }

    fn texts(row: &Row) -> (&str, &str, Option<&str>) {
        (&row.title, &row.subtitle, row.right.as_deref())
    }

    fn context_icons(model: &MenuModel) -> Vec<(&str, &str, Icon)> {
        let rows = model.contexts.as_ref().expect("a Context section");
        rows.iter().map(|context| (context.name.as_str(), context.row.subtitle.as_str(), context.row.icon)).collect()
    }

    /// Board Main.
    #[test]
    fn connected_shows_running_n_of_m_the_contexts_with_their_hosts_and_both_submenus() {
        let mut fixture = Fixture::new(online());
        fixture.turns.running = main_turns();
        fixture.selected = Some("staging");
        let model = fixture.model();
        assert_eq!(
            model.header,
            Header { title: "myapp worker".into(), subtitle: "Connected to prod".into(), on: true, enabled: true }
        );
        assert!(model.connected && model.bar.is_none() && model.failure.is_none() && !model.stop_now);
        let running = model.running.as_ref().expect("a Running section");
        assert_eq!((running.title, running.right.as_str()), ("Running", "3 of 4"));
        assert_eq!(
            running.rows.iter().map(|row| texts(&row.row)).collect::<Vec<_>>(),
            [
                ("reports/2026-q3", "GenerateReport · attempt 1", Some("0:42")),
                ("invoices/inv-10422", "Render · attempt 2", Some("0:07")),
                ("mailer/daily-digest", "Send · attempt 1", Some("0:03")),
            ]
        );
        assert!(running.rows.iter().all(|row| row.row.icon == Icon::Running));
        assert_eq!(
            context_icons(&model),
            [
                ("local", "127.0.0.1:8080", Icon::Context { selected: false }),
                ("prod", "actors.internal", Icon::Context { selected: true }),
                ("staging", "staging.actors.internal", Icon::Context { selected: false }),
            ],
            "the connected context is filled, not the pick"
        );
        let submenus = model.submenus.as_ref().expect("the submenus");
        assert_eq!(submenus.registered_right.as_deref(), Some("3 of 4"));
        assert_eq!(submenus.recent_right, None, "no failures: no text");
        assert!(model.connect.is_none());
        assert!(model.settings);
        assert_eq!(model.inspector.is_some(), cfg!(feature = "inspector"));
        assert_eq!((model.quit.as_str(), model.quit_enabled), ("Quit myapp", true));
    }

    #[test]
    fn running_time_is_minutes_and_seconds_and_clamps_at_zero() {
        let mut fixture = Fixture::new(online());
        fixture.turns.running =
            vec![running("work/a", "Work", 1, at(13, 20, 55)), running("work/b", "Work", 1, at(14, 33, 0))];
        let model = fixture.model();
        let rows = &model.running.as_ref().unwrap().rows;
        assert_eq!(rows[0].row.right.as_deref(), Some("71:05"));
        assert_eq!(rows[1].row.right.as_deref(), Some("0:00"));
        assert_eq!(rows[0].turn, fixture.turns.running[0], "the turn is the row's identity");
    }

    /// Board Registered.
    #[test]
    fn registered_types_mark_the_workers_types_with_their_running_counts() {
        let mut fixture = Fixture::new(online());
        fixture.turns.running = main_turns();
        let submenus = fixture.model().submenus.unwrap();
        assert_eq!(
            submenus.types,
            [
                TypeRow { name: "reports", runs: true, right: Some("1 running".into()) },
                TypeRow { name: "invoices", runs: true, right: Some("1 running".into()) },
                TypeRow { name: "mailer", runs: true, right: Some("1 running".into()) },
                TypeRow { name: "audit", runs: false, right: Some("Not on this worker".into()) },
            ]
        );
        assert_eq!(
            (ACTOR_TYPES, BACKGROUND_TASKS, CHOOSE_TYPES),
            ("Actor Types", ("Background Tasks", "Not available over HTTP"), "Choose Types…")
        );
    }

    fn finished(
        actor: &str,
        kind: &str,
        claimed_at: Timestamp,
        finished_at: Timestamp,
        outcome: TurnResult,
    ) -> FinishedTurn {
        FinishedTurn { turn: running(actor, kind, 1, claimed_at), finished_at, outcome }
    }

    /// Board Running, and one row per outcome.
    #[test]
    fn recent_turns_show_each_outcome_newest_first_and_count_failures() {
        let committed = TurnResult::Committed;
        let mut fixture = Fixture::new(online());
        let retry = |delivery| committed(TurnOutcome::Rescheduled { delivery, error: "deadline passed".into() });
        fixture.turns.finished = vec![
            finished("reports/2026-q1", "Archive", at(14, 26, 52), at(14, 27, 0), committed(TurnOutcome::Done)),
            finished("invoices/inv-10419", "Render", at(14, 28, 0), at(14, 29, 0), retry(Delivery::At(at(14, 40, 0)))),
            finished("mailer/welcome-881", "Send", at(14, 29, 58), at(14, 30, 0), committed(TurnOutcome::Done)),
            finished("reports/2026-q2", "GenerateReport", at(14, 29, 48), at(14, 31, 0), committed(TurnOutcome::Done)),
        ];
        let submenus = fixture.model().submenus.unwrap();
        assert_eq!(
            submenus.recent.iter().map(|row| (texts(&row.row), row.row.icon)).collect::<Vec<_>>(),
            [
                (("reports/2026-q2", "GenerateReport · done in 1:12", Some("14:31")), Icon::Done),
                (("mailer/welcome-881", "Send · done in 0:02", Some("14:30")), Icon::Done),
                (("invoices/inv-10419", "Render · deadline passed, retry 14:40", Some("14:29")), Icon::Failed),
                (("reports/2026-q1", "Archive · done in 0:08", Some("14:27")), Icon::Done),
            ]
        );
        assert_eq!(submenus.recent_right.as_deref(), Some("1 failed"), "board Main counts the retried turn");
        assert_eq!(RECENT, ("Finished on this worker", "Kept until the app quits"));

        let long = format!("first line {}\nsecond line", "x".repeat(100));
        fixture.turns.finished = vec![
            finished("a/1", "Work", at(14, 0, 0), at(14, 1, 0), committed(TurnOutcome::Failed(long))),
            finished("a/2", "Work", at(14, 0, 0), at(14, 2, 0), retry(Delivery::After(Duration::from_secs(300)))),
            finished("a/3", "Work", at(14, 0, 0), at(14, 3, 0), retry(Delivery::Now)),
            finished("a/4", "Work", at(14, 0, 0), at(14, 4, 0), committed(TurnOutcome::Cancelled)),
            finished("a/5", "Work", at(14, 0, 0), at(14, 5, 0), TurnResult::Abandoned),
        ];
        let submenus = fixture.model().submenus.unwrap();
        let rows: Vec<_> = submenus.recent.iter().map(|row| (row.row.subtitle.as_str(), row.row.icon)).collect();
        assert_eq!(rows[0], ("Work · abandoned", Icon::Failed));
        assert_eq!(rows[1], ("Work · cancelled", Icon::Cancelled));
        assert_eq!(rows[2], ("Work · deadline passed, retry now", Icon::Failed));
        assert_eq!(rows[3], ("Work · deadline passed, retry 14:07", Icon::Failed), "finish plus the delay");
        let failure = rows[4].0.strip_prefix("Work · ").unwrap();
        assert_eq!((failure.chars().count(), rows[4].1), (TURN_ERROR_LENGTH, Icon::Failed));
        assert!(failure.starts_with("first line xxx") && failure.ends_with('…') && !failure.contains("second"));
        assert_eq!(submenus.recent_right.as_deref(), Some("4 failed"), "failed, rescheduled and abandoned");
    }

    /// Board Disconnected, and the scenario "Not connected after a failure".
    #[test]
    fn not_connected_after_a_failure_shows_the_error_the_contexts_and_connect() {
        let mut fixture = Fixture::new(failed());
        fixture.turns.running = main_turns();
        fixture.failed_at = Some(at(14, 5, 30));
        let model = fixture.model();
        assert_eq!(
            model.header,
            Header { title: "myapp worker".into(), subtitle: "Not connected".into(), on: false, enabled: true }
        );
        let failure = model.failure.as_ref().expect("the failure row");
        assert_eq!(texts(failure), ("Couldn't reach staging", "Connection refused at 14:05", None));
        assert_eq!(failure.icon, Icon::Alert);
        assert!(model.submenus.is_none() && model.bar.is_none() && !model.connected);
        let stranded = model.running.as_ref().expect("turns a stop left behind");
        assert_eq!((stranded.title, stranded.right.as_str(), stranded.rows.len()), ("Finishing", "3", 3));
        assert_eq!(context_icons(&model)[1], ("prod", "actors.internal", Icon::Context { selected: true }));
        assert_eq!(model.connect.as_deref(), Some("Connect to prod"));
        assert!(model.settings && !model.stop_now);

        fixture.failed_at = None;
        fixture.turns.running.clear();
        assert!(fixture.model().running.is_none(), "no turns left: no section");
        assert_eq!(fixture.model().failure.unwrap().subtitle, "Connection refused", "no time recorded");
        fixture.state = TrayState::Offline;
        assert!(fixture.model().failure.is_none(), "offline has no failure row");
    }

    /// Scenario "Stop Now while the release is slow": the tray is offline while a cancelled turn's release is still in
    /// flight; the menu keeps the turn under Finishing until it ends, as `tests/control.rs` keeps it in the history.
    #[test]
    fn offline_after_stop_now_keeps_a_turn_left_behind_until_it_ends() {
        let mut fixture = Fixture::new(TrayState::Offline);
        fixture.ready = false;
        fixture.turns.running = vec![running("work/a", "Work", 1, at(14, 31, 0))];
        let model = fixture.model();
        let finishing = model.running.as_ref().expect("the turn left behind");
        assert_eq!((finishing.title, finishing.right.as_str()), ("Finishing", "1"));
        assert_eq!(texts(&finishing.rows[0].row), ("work/a", "Work · attempt 1", Some("1:00")));
        assert_eq!(finishing.rows[0].row.icon, Icon::Finishing);
        assert!(model.bar.is_none() && !model.stop_now && model.submenus.is_none());
        assert_eq!(model.header.subtitle, "Not connected");
        fixture.turns.running.clear();
        assert!(fixture.model().running.is_none(), "gone once it ended");
    }

    #[test]
    fn the_failure_row_shows_no_credentials_queries_or_control_characters() {
        let error = "error sending request for url (https://user:secret@actors.internal:8443/api/v1?token=hidden#x):\n\
                     connection\trefused\u{7}";
        let mut fixture = Fixture::new(TrayState::Failed { context: "prod".into(), error: error.into() });
        fixture.failed_at = Some(at(9, 1, 0));
        let failure = fixture.model().failure.unwrap();
        assert_eq!(
            failure.subtitle,
            "error sending request for url (https://actors.internal:8443/api/v1): connection refused at 09:01"
        );
        let long = format!("http://a@b/{}", "y".repeat(400));
        let shown = display_text(&long, FAILURE_LENGTH);
        assert_eq!(shown.chars().count(), FAILURE_LENGTH);
        assert!(shown.starts_with("http://b/yyy") && shown.ends_with('…'));
        assert_eq!(display_text("a://u@h?q b://h#f ok", 200), "a://h b://h ok");
        assert_eq!(
            display_text("for url (https://user:sec(ret)@host/path?token=abc(def)): refused", 200),
            "for url (https://host/path): refused"
        );
        assert_eq!(display_text("https://u:p?a#b@host/x?q=1", 200), "https://host/x", "a password holding ? and #");
        let cut = display_text(&format!("{} https://user:secret@host?token=abc", "z".repeat(190)), 200);
        assert!(!cut.contains("user") && !cut.contains("secret") && !cut.contains("tok"), "{cut}");
    }

    #[test]
    fn with_no_contexts_the_header_says_run_cicada_config_and_there_is_no_connect() {
        let mut fixture = Fixture::new(TrayState::Offline);
        fixture.contexts.clear();
        let model = fixture.model();
        assert_eq!(model.header.subtitle, "Not connected · Run cicada config");
        assert!(!model.header.enabled && model.connect.is_none());
        assert_eq!(model.contexts, Some(Vec::new()));

        let mut fixture = Fixture::new(TrayState::Offline);
        for selected in [None, Some("missing")] {
            fixture.selected = selected;
            let model = fixture.model();
            assert_eq!(model.header.subtitle, "Not connected");
            assert!(!model.header.enabled && model.connect.is_none(), "nothing selected: no target");
        }
    }

    /// The connecting menu, which has no board.
    #[test]
    fn connecting_is_main_without_running_rows() {
        let mut fixture = Fixture::new(TrayState::Connecting { context: "prod".into() });
        fixture.ready = false;
        fixture.turns.running = main_turns();
        let model = fixture.model();
        assert_eq!(model.header.subtitle, "Connecting to prod…");
        assert!(model.header.on && model.header.enabled && !model.connected);
        assert!(model.running.is_none());
        let submenus = model.submenus.as_ref().unwrap();
        assert_eq!(submenus.registered_right, None, "the worker's types are not known yet");
        assert!(submenus.types.iter().all(|row| !row.runs && row.right.is_none()));
        assert!(model.contexts.is_some() && model.connect.is_none() && model.settings);
    }

    /// Board Disconnecting, and the scenario "Disconnecting".
    #[test]
    fn disconnecting_shows_the_bar_the_finishing_turns_and_stop_now_only() {
        let now = Instant::now();
        // A 30 s grace period and a 10 s cancel timeout: the tray's deadline is 40 s after the stop began.
        let mut fixture = Fixture::new(stopping(now + Duration::from_secs(28)));
        fixture.turns.running = main_turns()[..2].to_vec();
        fixture.stopping = Some((now - Duration::from_secs(12), false));
        fixture.monotonic_now = now;
        let model = fixture.model();
        assert_eq!(
            model.header,
            Header {
                title: "myapp worker".into(),
                subtitle: "Disconnecting from prod…".into(),
                on: false,
                enabled: false
            }
        );
        let bar = model.bar.as_ref().expect("the bar");
        assert!((bar.fraction - 0.3).abs() < 1e-9);
        assert_eq!((bar.text, bar.left.as_deref()), ("Taking no new turns", Some("28 s left")));
        assert_eq!(bar.label, "Disconnecting: 12 of 40 seconds used");
        let finishing = model.running.as_ref().unwrap();
        assert_eq!((finishing.title, finishing.right.as_str()), ("Finishing", "2"));
        assert!(finishing.rows.iter().all(|row| row.row.icon == Icon::Finishing));
        assert!(model.stop_now);
        assert_eq!(STOP_NOW_HELP, "Stopping now cancels these turns. Their messages stay and run again later.");
        assert!(model.contexts.is_none() && model.submenus.is_none() && model.connect.is_none());
        assert!(model.inspector.is_none() && !model.settings && model.failure.is_none() && !model.connected);
        assert_eq!(model.quit, "Quit myapp");

        fixture.monotonic_now = now + Duration::from_millis(27_100);
        assert_eq!(fixture.model().bar.unwrap().left.as_deref(), Some("1 s left"), "rounded up");
        fixture.monotonic_now = now + Duration::from_secs(20);
        let bar = fixture.model().bar.unwrap();
        assert_eq!(
            bar.text, "Taking no new turns",
            "past the grace period, before the deadline: the worker's grace may still run"
        );
    }

    #[test]
    fn the_bar_is_full_with_no_grace_past_the_deadline_and_after_stop_now() {
        let now = Instant::now();
        let mut fixture = Fixture::new(stopping(now + Duration::from_secs(10)));
        fixture.monotonic_now = now;
        let full = |fixture: &Fixture| {
            let bar = fixture.model().bar.unwrap();
            (bar.fraction, bar.text, bar.left)
        };
        fixture.grace = Duration::ZERO;
        fixture.stopping = Some((now, false));
        assert_eq!(full(&fixture), (1.0, "Cancelling turns…", None), "zero grace");
        fixture.grace = Duration::from_secs(30);
        fixture.stopping = Some((now - Duration::from_secs(30), false));
        fixture.monotonic_now = now + Duration::from_secs(10);
        assert_eq!(full(&fixture), (1.0, "Cancelling turns…", None), "the deadline passed");
        fixture.monotonic_now = now;
        fixture.stopping = Some((now - Duration::from_secs(2), true));
        assert_eq!(full(&fixture), (1.0, "Cancelling turns…", None), "after Stop Now");
    }

    #[test]
    fn a_stop_before_the_worker_was_ready_shows_no_bar_and_no_stop_now() {
        let now = Instant::now();
        let mut fixture = Fixture::new(stopping(now));
        fixture.ready = false;
        fixture.stopping = Some((now, false));
        let model = fixture.model();
        assert!(model.bar.is_none() && !model.stop_now);
        assert_eq!(model.running.unwrap().right, "0");
    }

    #[test]
    fn rows_join_the_second_line_into_the_title_before_macos_14_4() {
        let row =
            Row { icon: Icon::Running, title: "work/a".into(), subtitle: "Work · attempt 2".into(), right: None };
        assert_eq!(row_text(&row, macos_subtitles((14, 2, 0)).unwrap()), ("work/a · Work · attempt 2".into(), None));
        assert_eq!(
            row_text(&row, macos_subtitles((14, 4, 0)).unwrap()),
            ("work/a".into(), Some("Work · attempt 2".into()))
        );
        let bare = Row { subtitle: String::new(), ..row };
        assert_eq!(row_text(&bare, false), ("work/a".into(), None));
    }

    #[test]
    fn a_quit_disables_the_switch_the_contexts_quit_and_the_inspector() {
        let mut fixture = Fixture::new(online());
        fixture.quitting = true;
        let model = fixture.model();
        assert!(!model.header.enabled && !model.contexts_enabled && !model.quit_enabled);
        if let Some(item) = model.inspector {
            assert_eq!((item.title, item.key, item.enabled), ("Open Inspector…", "i", false));
        }
    }

    #[test]
    fn open_inspector_is_feature_gated() {
        let model = Fixture::new(TrayState::Offline).model();
        if cfg!(feature = "inspector") {
            let item = model.inspector.expect("the feature adds the inspector item");
            assert_eq!((item.title, item.key, item.enabled), ("Open Inspector…", "i", true));
        } else {
            assert!(model.inspector.is_none());
        }
    }

    #[cfg(feature = "inspector")]
    #[test]
    fn the_inspector_uses_a_connection_in_any_phase_and_resolves_its_own_target_otherwise() {
        let control = RefCell::new(MenuControl::default());
        for (state, expected) in [
            (TrayState::Offline, None),
            (failed(), None),
            (TrayState::Connecting { context: "starting".into() }, Some("starting")),
            (online(), Some("prod")),
            (stopping(Instant::now()), Some("prod")),
        ] {
            let mut opened = false;
            InspectorItem::show(&control, &state, |context| {
                assert!(control.try_borrow_mut().is_ok(), "the host can reenter while opening");
                assert_eq!(context, expected);
                opened = true;
            });
            assert!(opened, "{state:?}");
        }
    }

    /// Scenario: A context pick after a failure.
    #[test]
    fn a_pick_while_not_connected_selects_without_connecting() {
        let names: Vec<String> = ["local", "prod", "staging"].map(String::from).into();
        for state in [failed(), TrayState::Offline] {
            let mut control = MenuControl::default();
            assert_eq!(control.selected(&names, Some("prod")), Some("prod"), "starts from the config file");
            assert_eq!(control.pick(&state, "local".into()), None, "no connection");
            assert_eq!(control.selected(&names, Some("prod")), Some("local"));
            assert_eq!(control.tick(&state), None);
            assert_eq!(
                control.toggle(&state, true, control.selected(&names, Some("prod"))),
                Some(Action::Connect("local".into())),
                "Connect and the switch use the pick"
            );
            assert_eq!(control.selected(&names, Some("prod")), Some("local"));
            let mut fixture = Fixture::new(state);
            fixture.selected = Some("local");
            let model = fixture.model();
            assert_eq!(model.connect.as_deref(), Some("Connect to local"));
            assert_eq!(context_icons(&model)[0].2, Icon::Context { selected: true });
        }
    }

    #[test]
    fn the_pick_survives_a_reload_while_its_name_exists() {
        let mut control = MenuControl::default();
        control.pick(&TrayState::Offline, "staging".into());
        let reloaded: Vec<String> = ["prod", "staging"].map(String::from).into();
        assert_eq!(control.selected(&reloaded, Some("prod")), Some("staging"));
        let without: Vec<String> = vec!["prod".into()];
        assert_eq!(control.selected(&without, Some("prod")), Some("prod"), "back to the config file's context");
        assert_eq!(control.selected(&without, Some("gone")), None);
    }

    #[test]
    fn a_switch_while_connected_becomes_the_pick() {
        let names: Vec<String> = ["prod", "test"].map(String::from).into();
        let mut control = MenuControl::default();
        assert_eq!(control.pick(&online(), "test".into()), Some(Action::Disconnect));
        assert_eq!(control.tick(&TrayState::Offline), Some(Action::Connect("test".into())));
        assert_eq!(control.selected(&names, Some("prod")), Some("test"));
    }

    #[test]
    fn macos_before_14_is_refused_with_exit_code_1() {
        assert_eq!(
            macos_subtitles((13, 6, 0)),
            Err(("the Cicada menu bar app requires macOS 14 or later", ExitCode::from(1)))
        );
        assert_eq!(macos_subtitles((14, 0, 0)), Ok(false));
        assert_eq!(macos_subtitles((14, 3, 9)), Ok(false));
        assert_eq!(macos_subtitles((15, 0, 0)), Ok(true));
    }

    #[test]
    fn quit_disconnects_once_and_waits_until_offline() {
        let mut control = MenuControl::default();
        assert_eq!(control.quit(&online()), Some(Action::Disconnect));
        assert_eq!(control.quit(&online()), None);
        assert_eq!(control.tick(&stopping(Instant::now())), None);
        assert_eq!(control.tick(&TrayState::Offline), Some(Action::StopApp));
        assert_eq!(control.tick(&TrayState::Offline), None);
    }

    #[test]
    fn quit_waits_for_a_stop_already_in_progress_and_ends_even_if_it_fails() {
        let mut control = MenuControl::default();
        let state = stopping(Instant::now());
        assert_eq!(control.quit(&state), None);
        assert_eq!(control.tick(&state), None);
        assert_eq!(control.tick(&failed()), Some(Action::StopApp));
    }

    #[test]
    fn quit_from_failed_needs_no_disconnect() {
        let mut control = MenuControl::default();
        assert_eq!(control.quit(&failed()), None);
        assert_eq!(control.tick(&failed()), Some(Action::StopApp));
    }

    #[test]
    fn system_termination_defers_until_the_connection_ends() {
        let mut control = MenuControl::default();
        assert_eq!(control.terminate(&online()), Terminate::Later(Some(Action::Disconnect)));
        assert_eq!(control.terminate(&online()), Terminate::Later(None));
        assert_eq!(control.tick(&stopping(Instant::now())), None);
        assert_eq!(control.tick(&TrayState::Offline), Some(Action::ReplyToTerminate));
    }

    #[test]
    fn every_quit_request_closes_the_inspector_before_letting_the_host_act() {
        for (request, state, expected) in [
            (QuitRequest::Local, online(), Terminate::Later(Some(Action::Disconnect))),
            (QuitRequest::Local, TrayState::Offline, Terminate::Later(None)),
            (QuitRequest::System, online(), Terminate::Later(Some(Action::Disconnect))),
            (QuitRequest::System, stopping(Instant::now()), Terminate::Later(None)),
            (QuitRequest::System, TrayState::Offline, Terminate::Now),
            (QuitRequest::System, failed(), Terminate::Now),
        ] {
            let control = RefCell::new(MenuControl::default());
            let mut inspector_open = true;
            let decision = request.begin(&control, &state, || {
                assert!(control.try_borrow_mut().is_ok(), "the host can reenter while closing");
                assert!(control.borrow().quitting(), "quitting is set before close");
                #[cfg(feature = "inspector")]
                InspectorItem::show(&control, &state, |_| panic!("a reentrant show must be blocked"));
                inspector_open = false;
            });
            assert!(!inspector_open);
            assert_eq!(decision, expected);
        }
    }

    #[test]
    fn a_system_terminate_during_quit_still_replies_to_appkit() {
        let mut control = MenuControl::default();
        assert_eq!(control.quit(&online()), Some(Action::Disconnect));
        assert_eq!(control.terminate(&stopping(Instant::now())), Terminate::Later(None));
        assert_eq!(control.tick(&failed()), Some(Action::ReplyToTerminate));
    }

    #[test]
    fn a_later_pick_while_stopping_replaces_the_pending_context() {
        let mut control = MenuControl::default();
        assert_eq!(control.pick(&online(), "prod".into()), None);
        assert_eq!(control.pick(&online(), "a".into()), Some(Action::Disconnect));
        let state = stopping(Instant::now());
        assert_eq!(control.pick(&state, "b".into()), None);
        assert_eq!(control.tick(&state), None);
        assert_eq!(control.tick(&failed()), Some(Action::Connect("b".into())));
        assert_eq!(control.tick(&TrayState::Offline), None);
    }

    #[test]
    fn quit_clears_a_pending_pick_and_refuses_new_ones() {
        let mut control = MenuControl::default();
        assert_eq!(control.pick(&online(), "test".into()), Some(Action::Disconnect));
        let state = stopping(Instant::now());
        assert_eq!(control.quit(&state), None);
        assert!(control.pending.is_none());
        assert_eq!(control.pick(&state, "other".into()), None);
        assert_eq!(control.toggle(&TrayState::Offline, true, Some("test")), None);
        assert_eq!(control.tick(&TrayState::Offline), Some(Action::StopApp));
    }

    /// Scenario: Apply while connected.
    #[test]
    fn apply_while_online_reconnects_to_the_same_or_another_context() {
        for name in ["prod", "test"] {
            let mut control = MenuControl::default();
            assert_eq!(control.apply(&online(), name.into()), Some(Action::Disconnect));
            let state = stopping(Instant::now());
            assert_eq!(control.tick(&state), None);
            assert_eq!(control.tick(&TrayState::Offline), Some(Action::Connect(name.into())));
        }
        let mut control = MenuControl::default();
        let connecting = TrayState::Connecting { context: "prod".into() };
        assert_eq!(control.apply(&connecting, "prod".into()), Some(Action::Disconnect));
    }

    #[test]
    fn apply_while_stopping_replaces_the_pending_pick() {
        let mut control = MenuControl::default();
        assert_eq!(control.pick(&online(), "test".into()), Some(Action::Disconnect));
        let state = stopping(Instant::now());
        assert_eq!(control.apply(&state, "prod".into()), None);
        assert_eq!(control.tick(&failed()), Some(Action::Connect("prod".into())));
    }

    #[test]
    fn apply_offline_only_clears_a_pending_pick() {
        let mut control = MenuControl { pending: Some("test".into()), ..MenuControl::default() };
        assert_eq!(control.apply(&TrayState::Offline, "prod".into()), None);
        assert!(control.pending.is_none());
        assert_eq!(control.tick(&TrayState::Offline), None);
        assert_eq!(control.apply(&failed(), "prod".into()), None);
    }

    #[test]
    fn apply_during_a_quit_does_nothing() {
        let mut control = MenuControl::default();
        assert_eq!(control.quit(&online()), Some(Action::Disconnect));
        let state = stopping(Instant::now());
        assert_eq!(control.apply(&state, "prod".into()), None);
        assert!(control.pending.is_none());
        assert_eq!(control.tick(&TrayState::Offline), Some(Action::StopApp));
    }
}
