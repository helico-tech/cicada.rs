use std::process::ExitCode;

use cicada::store::TurnOutcome;
use cicada::time::{Timestamp, between};
use tokio::time::Instant;

use crate::{FinishedTurn, RunningTurn, TrayState, TurnResult, TurnSnapshot};

pub(crate) struct MenuInput<'a> {
    pub title: &'a str,
    pub state: &'a TrayState,
    pub turns: &'a TurnSnapshot,
    pub registered_types: &'a [&'static str],
    pub worker_types: &'a [&'static str],
    /// Whether the connection's worker was ready.
    pub ready: bool,
    pub contexts: &'a [String],
    pub current: Option<&'a str>,
    pub now: Timestamp,
    pub monotonic_now: Instant,
    pub quitting: bool,
}

#[derive(PartialEq, Eq)]
pub(crate) struct MenuModel {
    pub header: Header,
    /// Offer Stop Now: while stopping a worker that was ready.
    pub stop_now: bool,
    pub dimmed: bool,
    pub running: Vec<RunningRow>,
    pub contexts: Vec<Choice<String>>,
    pub contexts_enabled: bool,
    pub registered_types: Vec<Choice<&'static str>>,
    pub recent: Vec<RecentRow>,
    pub failures: usize,
}

#[derive(PartialEq, Eq)]
pub(crate) struct Header {
    pub title: String,
    pub subtitle: String,
    pub error: Option<String>,
    pub on: bool,
    pub enabled: bool,
}

#[derive(PartialEq, Eq)]
pub(crate) struct Choice<T> {
    pub name: T,
    pub marked: bool,
}

#[derive(PartialEq, Eq)]
pub(crate) struct RunningRow {
    pub turn: RunningTurn,
    pub elapsed: u64,
}

pub(crate) fn running_row_text(row: &RunningRow, subtitles: bool) -> (String, Option<String>) {
    let title = format!("{}     {}s", row.turn.actor, row.elapsed);
    let details = format!("{} · Attempt {}", row.turn.message_type, row.turn.attempt);
    if subtitles { (title, Some(details)) } else { (format!("{title} · {details}"), None) }
}

/// Checks the minimum macOS version and reports whether it supports menu item subtitles.
pub(crate) fn macos_subtitles(version: (isize, isize, isize)) -> Result<bool, (&'static str, ExitCode)> {
    if version < (14, 0, 0) {
        return Err(("the Cicada menu bar app requires macOS 14 or later", ExitCode::FAILURE));
    }
    Ok(version >= (14, 4, 0))
}

#[derive(PartialEq, Eq)]
pub(crate) struct RecentRow {
    pub turn: FinishedTurn,
    pub outcome: &'static str,
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

pub(crate) fn menu(input: &MenuInput<'_>) -> MenuModel {
    let current = input.current.filter(|name| input.contexts.iter().any(|context| context == name));
    let marked = input.state.connection().or(current);
    let (subtitle, error, on, enabled) = match input.state {
        TrayState::Offline => {
            let title = if current.is_some() { "Offline" } else { "Offline · Run cicada config" };
            (title.to_owned(), None, false, current.is_some())
        }
        TrayState::Connecting { context } => (format!("Connecting to {context}"), None, true, true),
        TrayState::Online { context } => (format!("Connected to {context}"), None, true, true),
        TrayState::Stopping { until, .. } => {
            let left = until.saturating_duration_since(input.monotonic_now);
            let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            (format!("Disconnecting · {seconds}s left"), None, false, false)
        }
        TrayState::Failed { context, error } => {
            (format!("Connection failed · {context}"), Some(error.clone()), false, current.is_some())
        }
    };
    let mut failures = 0;
    let recent = input
        .turns
        .finished
        .iter()
        .rev()
        .map(|turn| {
            let outcome = match &turn.outcome {
                TurnResult::Committed(TurnOutcome::Done) => "Done",
                TurnResult::Committed(TurnOutcome::Cancelled) => "Cancelled",
                TurnResult::Committed(TurnOutcome::Rescheduled { .. }) => "Rescheduled",
                TurnResult::Committed(TurnOutcome::Failed(_)) => {
                    failures += 1;
                    "Failed"
                }
                TurnResult::Abandoned => {
                    failures += 1;
                    "Abandoned"
                }
            };
            RecentRow { turn: turn.clone(), outcome }
        })
        .collect();

    MenuModel {
        header: Header { title: input.title.to_owned(), subtitle, error, on, enabled: enabled && !input.quitting },
        stop_now: input.ready && matches!(input.state, TrayState::Stopping { .. }),
        dimmed: input.state.connection().is_none(),
        running: input
            .turns
            .running
            .iter()
            .map(|turn| RunningRow { turn: turn.clone(), elapsed: between(turn.claimed_at, input.now).as_secs() })
            .collect(),
        contexts: input
            .contexts
            .iter()
            .map(|name| Choice { name: name.clone(), marked: Some(name.as_str()) == marked })
            .collect(),
        contexts_enabled: !input.quitting,
        registered_types: input
            .registered_types
            .iter()
            .map(|&name| Choice {
                name,
                marked: input.state.connection().is_some() && input.worker_types.contains(&name),
            })
            .collect(),
        recent,
        failures,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quit {
    App,
    System,
    Finishing,
}

#[derive(Default)]
pub(crate) struct MenuControl {
    pending: Option<String>,
    quit: Option<Quit>,
}

impl MenuControl {
    pub fn quitting(&self) -> bool {
        self.quit.is_some()
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

    pub fn pick(&mut self, state: &TrayState, name: String) -> Option<Action> {
        if self.quitting() {
            return None;
        }
        match state {
            TrayState::Offline | TrayState::Failed { .. } => {
                self.pending = None;
                Some(Action::Connect(name))
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
            None => self.pending.take().map(Action::Connect),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cicada::time::from_micros;

    use super::*;

    fn model(state: &TrayState, now: Instant, turns: &TurnSnapshot, current: Option<&str>) -> MenuModel {
        model_of(state, true, now, turns, current)
    }

    fn model_of(
        state: &TrayState,
        ready: bool,
        now: Instant,
        turns: &TurnSnapshot,
        current: Option<&str>,
    ) -> MenuModel {
        menu(&MenuInput {
            title: "Test Worker",
            state,
            turns,
            registered_types: &["one", "two", "three"],
            worker_types: &["one", "three"],
            ready,
            contexts: &["prod".into(), "test".into()],
            current,
            now: from_micros(4_000_000),
            monotonic_now: now,
            quitting: false,
        })
    }

    fn online() -> TrayState {
        TrayState::Online { context: "prod".into() }
    }

    fn failed() -> TrayState {
        TrayState::Failed { context: "prod".into(), error: "Server unavailable".into() }
    }

    fn stopping(until: Instant) -> TrayState {
        TrayState::Stopping { context: "prod".into(), until }
    }

    #[test]
    fn every_state_gives_the_header_and_switch() {
        let now = Instant::now();
        let turns = TurnSnapshot::default();
        let connecting = TrayState::Connecting { context: "prod".into() };
        for (state, subtitle, on, enabled, dimmed) in [
            (TrayState::Offline, "Offline", false, true, true),
            (connecting, "Connecting to prod", true, true, false),
            (online(), "Connected to prod", true, true, false),
            (stopping(now + Duration::from_millis(3900)), "Disconnecting · 4s left", false, false, false),
            (stopping(now - Duration::from_secs(1)), "Disconnecting · 0s left", false, false, false),
            (failed(), "Connection failed · prod", false, true, true),
        ] {
            let menu = model(&state, now, &turns, Some("test"));
            assert_eq!(menu.header.title, "Test Worker");
            assert_eq!(menu.header.subtitle, subtitle);
            assert_eq!((menu.header.on, menu.header.enabled, menu.dimmed), (on, enabled, dimmed));
        }
    }

    #[test]
    fn stop_now_is_offered_only_while_stopping_a_ready_worker() {
        let now = Instant::now();
        let turns = TurnSnapshot::default();
        let connecting = TrayState::Connecting { context: "prod".into() };
        for state in [TrayState::Offline, connecting, online(), failed()] {
            assert!(!model(&state, now, &turns, Some("prod")).stop_now, "{state:?}");
        }
        let state = stopping(now + Duration::from_secs(3));
        assert!(model(&state, now, &turns, Some("prod")).stop_now);
        assert!(!model_of(&state, false, now, &turns, Some("prod")).stop_now, "never ready");
    }

    #[test]
    fn no_current_context_disables_the_offline_switch() {
        for current in [None, Some("missing")] {
            let menu = model(&TrayState::Offline, Instant::now(), &TurnSnapshot::default(), current);
            assert!(!menu.header.enabled);
            assert!(menu.header.subtitle.contains("cicada config"));
        }
    }

    #[test]
    fn the_connected_context_and_the_workers_types_are_marked() {
        let turns = TurnSnapshot::default();
        let now = Instant::now();
        let menu = model(&online(), now, &turns, Some("test"));
        assert_eq!(menu.contexts.iter().map(|row| row.marked).collect::<Vec<_>>(), [true, false]);
        assert_eq!(menu.registered_types.iter().map(|row| row.marked).collect::<Vec<_>>(), [true, false, true]);
        let offline = model(&TrayState::Offline, now, &turns, Some("test"));
        assert_eq!(offline.contexts.iter().map(|row| row.marked).collect::<Vec<_>>(), [false, true]);
        assert!(offline.registered_types.iter().all(|row| !row.marked));
    }

    #[test]
    fn running_turns_show_the_claim_details_and_clamp_time_at_zero() {
        let running = RunningTurn {
            actor: "work/a".parse().unwrap(),
            message_type: "Work".into(),
            attempt: 2,
            claimed_at: from_micros(500_000),
        };
        let future = RunningTurn { claimed_at: from_micros(5_000_000), ..running.clone() };
        let turns = TurnSnapshot { running: vec![running.clone(), future], ..Default::default() };
        let menu = model(&online(), Instant::now(), &turns, Some("prod"));
        assert_eq!(menu.running[0].turn, running);
        assert_eq!(menu.running[0].elapsed, 3);
        assert_eq!(menu.running[1].elapsed, 0);
    }

    #[test]
    fn running_turn_text_puts_the_details_in_the_title_before_macos_14_4() {
        let row = RunningRow {
            turn: RunningTurn {
                actor: "work/a".parse().unwrap(),
                message_type: "Work".into(),
                attempt: 2,
                claimed_at: from_micros(0),
            },
            elapsed: 3,
        };
        assert_eq!(
            running_row_text(&row, macos_subtitles((14, 2, 0)).unwrap()),
            ("work/a     3s · Work · Attempt 2".into(), None)
        );
        assert_eq!(
            running_row_text(&row, macos_subtitles((14, 4, 0)).unwrap()),
            ("work/a     3s".into(), Some("Work · Attempt 2".into()))
        );
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
    fn recent_turns_are_newest_first_and_the_badge_counts_failures_and_abandonment() {
        let turn = RunningTurn {
            actor: "work/a".parse().unwrap(),
            message_type: "Work".into(),
            attempt: 1,
            claimed_at: from_micros(0),
        };
        let finished = [
            TurnResult::Committed(TurnOutcome::Failed("failed".into())),
            TurnResult::Committed(TurnOutcome::Done),
            TurnResult::Abandoned,
            TurnResult::Committed(TurnOutcome::Cancelled),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, outcome)| FinishedTurn { turn: turn.clone(), finished_at: from_micros(i as i64), outcome })
        .collect();
        let turns = TurnSnapshot { running: Vec::new(), finished };
        let menu = model(&online(), Instant::now(), &turns, Some("prod"));
        assert_eq!(
            menu.recent.iter().map(|row| row.outcome).collect::<Vec<_>>(),
            ["Cancelled", "Abandoned", "Done", "Failed"]
        );
        assert_eq!(menu.failures, 2);
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
        assert_eq!(control.terminate(&TrayState::Offline), Terminate::Now);
        assert_eq!(control.terminate(&failed()), Terminate::Now);
        assert_eq!(control.terminate(&online()), Terminate::Later(Some(Action::Disconnect)));
        assert_eq!(control.terminate(&online()), Terminate::Later(None));
        assert_eq!(control.tick(&stopping(Instant::now())), None);
        assert_eq!(control.tick(&TrayState::Offline), Some(Action::ReplyToTerminate));
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
    fn a_pick_from_failed_connects_at_once() {
        let mut control = MenuControl::default();
        assert_eq!(control.pick(&failed(), "test".into()), Some(Action::Connect("test".into())));
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
