use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use cicada::ActorAddress;
use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
use cicada::store::TurnOutcome;
use cicada::time::Timestamp;
use tokio::sync::watch;

const FINISHED_LIMIT: usize = 50;

/// A turn the worker has started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningTurn {
    /// The actor doing the work.
    pub actor: ActorAddress,
    /// The type of the claimed message.
    pub message_type: String,
    /// Number of times the message has been claimed, including this turn.
    pub attempt: u32,
    /// Store time of the claim.
    pub claimed_at: Timestamp,
}

/// How an observed turn finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnResult {
    /// The worker saw the server commit the turn with this outcome.
    Committed(TurnOutcome),
    /// The worker saw no commit succeed, such as when shutdown cancelled a running handler.
    Abandoned,
}

/// A turn that left the running list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishedTurn {
    /// The turn as it was while running.
    pub turn: RunningTurn,
    /// Local wall-clock time when the observer recorded the end.
    pub finished_at: Timestamp,
    /// What happened to the turn.
    pub outcome: TurnResult,
}

/// The worker's running turns and up to 50 recently finished turns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnSnapshot {
    /// Running turns, in the order the observer saw them start.
    pub running: Vec<RunningTurn>,
    /// Finished turns, oldest first.
    pub finished: Vec<FinishedTurn>,
}

#[derive(Clone)]
pub(crate) struct TurnHistory {
    book: Arc<Mutex<Book>>,
    changed: watch::Sender<u64>,
}

#[derive(Default)]
struct Book {
    next_id: u64,
    running: BTreeMap<u64, RunningTurn>,
    finished: VecDeque<FinishedTurn>,
}

impl Default for TurnHistory {
    fn default() -> Self {
        Self { book: Arc::default(), changed: watch::channel(0).0 }
    }
}

impl TurnHistory {
    pub(crate) fn snapshot(&self) -> TurnSnapshot {
        let book = self.book.lock().expect("tray turn history");
        TurnSnapshot {
            running: book.running.values().cloned().collect(),
            finished: book.finished.iter().cloned().collect(),
        }
    }

    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn changed(&self) {
        self.changed.send_modify(|count| *count = count.wrapping_add(1));
    }
}

impl TurnObserver for TurnHistory {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        let running = RunningTurn {
            actor: turn.actor().clone(),
            message_type: turn.message().message_type.clone(),
            attempt: turn.message().attempt,
            claimed_at: turn.claimed_at(),
        };
        let mut book = self.book.lock().expect("tray turn history");
        let id = book.next_id;
        book.next_id = id.wrapping_add(1);
        book.running.insert(id, running);
        drop(book);
        self.changed();
        Box::new(Record { history: self.clone(), id: Some(id) })
    }
}

struct Record {
    history: TurnHistory,
    id: Option<u64>,
}

impl Record {
    fn finish(&mut self, outcome: TurnResult) {
        let Some(id) = self.id.take() else { return };
        let mut book = self.history.book.lock().expect("tray turn history");
        let turn = book.running.remove(&id).expect("the turn is still running");
        if book.finished.len() == FINISHED_LIMIT {
            book.finished.pop_front();
        }
        book.finished.push_back(FinishedTurn { turn, finished_at: SystemTime::now().into(), outcome });
        drop(book);
        self.history.changed();
    }
}

impl ObservedTurn for Record {
    fn turn_ended(mut self: Box<Self>, end: &TurnEnd<'_>) {
        self.finish(TurnResult::Committed(end.outcome().clone()));
    }
}

impl Drop for Record {
    fn drop(&mut self) {
        self.finish(TurnResult::Abandoned);
    }
}
