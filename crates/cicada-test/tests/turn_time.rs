//! `now` is the claim time; `current_time()` moves on during the turn; a delay counts from the commit.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::prelude::*;
use cicada::store::Clock;
use cicada::time::{self, Timestamp};
use cicada_test::{TestActors, TokioClock, default_start};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pacer {
    id: String,
}

impl ActorRef for Pacer {
    const TYPE: &'static str = "pacer";
    type Message = PacerMsg;
}

fn pacer() -> Pacer {
    Pacer { id: "p".into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum PacerMsg {
    /// Works for `seconds`, then sends `Next` one minute later: at `now + 1 min` or at `current_time() + 1 min`.
    Work {
        seconds: u64,
        #[serde(rename = "fromEnd")]
        from_end: bool,
    },
    Next,
}

#[derive(Default, Clone)]
struct Seen {
    /// (now, current_time()) at the end of a Work turn.
    work: Arc<Mutex<Vec<(Timestamp, Timestamp)>>>,
    /// deliver_at of each handled Next.
    next_due: Arc<Mutex<Vec<Timestamp>>>,
}

struct PacerActor(Seen);

impl Actor for PacerActor {
    type Ref = Pacer;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: PacerMsg) -> HandlerResult {
        match message {
            PacerMsg::Work { seconds, from_end } => {
                tokio::time::sleep(Duration::from_secs(seconds)).await; // virtual time
                self.0.work.lock().unwrap().push((turn.now(), turn.current_time()));
                let base = if from_end { turn.current_time() } else { turn.now() };
                let at = time::add(base, Duration::from_secs(60))?;
                turn.send_self_with(PacerMsg::Next, SendOptions::new().at(at))?;
            }
            PacerMsg::Next => self.0.next_due.lock().unwrap().push(turn.message().deliver_at),
        }
        Ok(())
    }
}

async fn harness(seen: &Seen) -> TestActors {
    let actor = PacerActor(seen.clone());
    let test = TestActors::builder().with(move |n| n.host(actor)).build().await.unwrap();
    test.create(&pacer(), &NoState {}).await;
    test
}

fn plus(t: Timestamp, seconds: u64) -> Timestamp {
    time::add(t, Duration::from_secs(seconds)).unwrap()
}

#[tokio::test(start_paused = true)]
async fn current_time_moves_on_during_a_turn_while_now_stays_at_the_claim() {
    let seen = Seen::default();
    let test = harness(&seen).await;
    let start = test.now();
    test.send(&pacer(), PacerMsg::Work { seconds: 70, from_end: true }).await;
    test.run_until_idle().await;
    let (claimed_at, current) = seen.work.lock().unwrap()[0];
    assert_eq!(claimed_at, start);
    assert_eq!(time::between(claimed_at, current), Duration::from_secs(70));
    // Scheduled from the end of the work: one minute after the commit.
    let pending = test.pending(&pacer()).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].deliver_at, plus(start, 70 + 60));
}

#[tokio::test(start_paused = true)]
async fn an_at_time_from_now_in_a_long_turn_is_due_at_the_commit_without_the_pause() {
    let seen = Seen::default();
    let test = harness(&seen).await;
    let start = test.now();
    test.send(&pacer(), PacerMsg::Work { seconds: 70, from_end: false }).await;
    test.run_until_idle().await;
    // now + 1 min lies before the commit (70 s): clamped to the commit, so there is no pause at all.
    assert_eq!(*seen.next_due.lock().unwrap(), vec![plus(start, 70)]);
}

/// A dependency with a clock and a monotonic time source of its own, as a rate limiter would have.
struct Dependent {
    clock: TokioClock,
    created: tokio::time::Instant,
    readings: Arc<Mutex<Vec<Timestamp>>>,
    waited: Arc<Mutex<Vec<Duration>>>,
}

impl Actor for Dependent {
    type Ref = Pacer;
    type State = NoState;

    async fn handle(&self, _turn: &mut Turn<Self>, message: PacerMsg) -> HandlerResult {
        if let PacerMsg::Work { .. } = message {
            self.readings.lock().unwrap().push(time::truncate(self.clock.now()));
            self.waited.lock().unwrap().push(self.created.elapsed());
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn types_can_be_built_with_the_tests_virtual_clock_and_time_source() {
    // The test's virtual time is tokio's paused clock, so a dependency
    // built on it (a `TokioClock`, `tokio::time::Instant`) shares the store's time.
    let readings = Arc::new(Mutex::new(Vec::new()));
    let waited = Arc::new(Mutex::new(Vec::new()));
    let actor = Dependent {
        clock: TokioClock::new(default_start()),
        created: tokio::time::Instant::now(),
        readings: readings.clone(),
        waited: waited.clone(),
    };
    let test = TestActors::builder().with(move |n| n.host(actor)).build().await.unwrap();
    test.create(&pacer(), &NoState {}).await;
    test.advance_time_by(Duration::from_secs(300)).await;
    test.send(&pacer(), PacerMsg::Work { seconds: 0, from_end: false }).await;
    test.run_until_idle().await;
    assert_eq!(*readings.lock().unwrap(), vec![test.now()], "the dependency's clock is the store's virtual clock");
    assert_eq!(*waited.lock().unwrap(), vec![Duration::from_secs(300)], "and its time source tokio's");
}
