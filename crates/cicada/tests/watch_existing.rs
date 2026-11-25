//! A watch of an actor that must exist, on a node in virtual time (ADR 0063): `Turn::watch_existing` frees a pool's
//! place for a run that went before the pool's turn, where a plain `Turn::watch` never hears of it, and refuses a type.
//! Public API only.

use std::collections::BTreeSet;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::testing::TestActors;
use cicada::{Failed, StopCause, Stopped};
use serde::{Deserialize, Serialize};

/// A pool with places for runs: it gives a place to a run, then watches the run to free the place when it goes.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pool;

impl ActorRef for Pool {
    const TYPE: &'static str = "pool";
    type Message = PoolMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
enum PoolMsg {
    Failed(Failed),
    Stopped(Stopped),
    /// Takes a place for the run and starts it.
    Give {
        run: String,
    },
    /// Watches the run with `watch_existing`, or with `watch` when `plain`.
    Track {
        run: String,
        plain: bool,
    },
    /// Tries `watch_existing` of the target and notes how the call ended.
    TryTrack {
        target: String,
    },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Places {
    taken: BTreeSet<String>,
    stopped: Vec<Stopped>,
    calls: Vec<String>,
}

struct PoolActor;

impl Actor for PoolActor {
    type Ref = Pool;
    type State = Places;

    async fn handle(&self, turn: &mut Turn<Self>, message: PoolMsg) -> HandlerResult {
        match message {
            PoolMsg::Failed(_) => {}
            PoolMsg::Stopped(stopped) => {
                turn.state_mut().taken.remove(stopped.actor.key());
                turn.state_mut().stopped.push(stopped);
            }
            PoolMsg::Give { run } => {
                turn.state_mut().taken.insert(run.clone());
                turn.send(&Run { id: run }, RunMsg::Start)?;
            }
            PoolMsg::Track { run, plain: false } => turn.watch_existing(&Run { id: run })?,
            PoolMsg::Track { run, plain: true } => turn.watch(&Run { id: run })?,
            PoolMsg::TryTrack { target } => {
                let call = match turn.watch_existing(target.as_str()) {
                    Ok(()) => "watching".to_owned(),
                    Err(e) => format!("refused: {e}"),
                };
                turn.state_mut().calls.push(call);
            }
        }
        Ok(())
    }
}

/// A run: started by the pool, and it stops when told.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Run {
    id: String,
}

impl ActorRef for Run {
    const TYPE: &'static str = "run";
    type Message = RunMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum RunMsg {
    Start,
    Stop,
}

struct RunActor;

impl Actor for RunActor {
    type Ref = Run;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: RunMsg) -> HandlerResult {
        if let RunMsg::Stop = message {
            turn.stop();
        }
        Ok(())
    }
}

fn run(key: &str) -> Run {
    Run { id: key.to_owned() }
}

async fn node() -> TestActors {
    let test = TestActors::builder().with(|node| node.host(PoolActor).host(RunActor)).build().await.unwrap();
    test.create(&Pool, &Places::default()).await;
    test
}

/// Gives a place to the run `key` and deletes the run before the pool's next turn.
async fn give_and_delete(test: &TestActors, key: &str) {
    test.create(&run(key), &NoState {}).await;
    test.send(&Pool, PoolMsg::Give { run: key.into() }).await;
    test.run_until_idle().await;
    assert!(test.state::<_, Places>(&Pool).await.taken.contains(key), "the place is taken");
    test.system().delete_actor(&run(key).address().unwrap()).await.unwrap();
}

/// Scenario: A pool place is freed.
#[tokio::test(start_paused = true)]
async fn a_run_deleted_before_the_pools_turn_frees_its_place_through_watch_existing() {
    let test = node().await;
    give_and_delete(&test, "r1").await;
    test.send(&Pool, PoolMsg::Track { run: "r1".into(), plain: false }).await;
    test.run_until_idle().await;
    let places = test.state::<_, Places>(&Pool).await;
    assert!(places.taken.is_empty(), "the place is freed: {places:?}");
    let gone = run("r1").address().unwrap();
    assert_eq!(places.stopped, [Stopped { actor: gone, cause: StopCause::Deleted, by: None }]);
    assert!(test.system().watches(None, None, None, 10).await.unwrap().items.is_empty(), "no watch on nothing");
}

/// A plain watch of the same run lands on nothing: the place stays taken (Factory issue 0014).
#[tokio::test(start_paused = true)]
async fn a_plain_watch_of_a_run_that_is_gone_keeps_the_place() {
    let test = node().await;
    give_and_delete(&test, "r1").await;
    test.send(&Pool, PoolMsg::Track { run: "r1".into(), plain: true }).await;
    test.run_until_idle().await;
    let places = test.state::<_, Places>(&Pool).await;
    assert_eq!(places.taken.iter().collect::<Vec<_>>(), ["r1"]);
    assert!(places.stopped.is_empty());
}

/// Scenario: The target is there.
#[tokio::test(start_paused = true)]
async fn a_run_that_is_there_is_watched_and_its_stop_frees_the_place() {
    let test = node().await;
    test.create(&run("r2"), &NoState {}).await;
    test.send(&Pool, PoolMsg::Give { run: "r2".into() }).await;
    test.send(&Pool, PoolMsg::Track { run: "r2".into(), plain: false }).await;
    test.run_until_idle().await;
    assert_eq!(test.system().watches(None, None, None, 10).await.unwrap().items.len(), 1, "a plain watch");
    assert!(test.state::<_, Places>(&Pool).await.stopped.is_empty(), "nothing while it runs");
    test.send(&run("r2"), RunMsg::Stop).await;
    test.run_until_idle().await;
    let places = test.state::<_, Places>(&Pool).await;
    assert!(places.taken.is_empty());
    let actor = run("r2").address().unwrap();
    assert_eq!(places.stopped, [Stopped { actor: actor.clone(), cause: StopCause::Stopped, by: Some(actor) }]);
}

/// Scenario: A type is refused.
#[tokio::test(start_paused = true)]
async fn watch_existing_of_a_type_or_every_type_fails_at_the_call_and_the_turn_goes_on() {
    let test = node().await;
    let first = test.send(&Pool, PoolMsg::TryTrack { target: "billing".into() }).await;
    test.send(&Pool, PoolMsg::TryTrack { target: "*".into() }).await;
    test.run_until_idle().await;
    let calls = test.state::<_, Places>(&Pool).await.calls;
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[0].starts_with("refused: watch_existing takes one actor, not billing"), "{calls:?}");
    assert!(calls[1].starts_with("refused: watch_existing takes one actor, not *"), "{calls:?}");
    assert_eq!(test.outcome(&first).await.unwrap().status, OutcomeStatus::Done, "the turn can go on");
    assert!(test.system().watches(None, None, None, 10).await.unwrap().items.is_empty());
}
