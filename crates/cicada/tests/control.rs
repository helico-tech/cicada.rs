//! Control effects on a node, in virtual time (ADR 0024): `Turn::pause`, `resume`, `purge` and `delete`, by ref and by
//! address, applied by the turn's commit with its other effects or not at all, in the order of the commit, and with
//! the meaning of the same calls from outside a turn. Public API only.

use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::NodeConfig;
use cicada::store::{Message, OutcomeRecord, OutcomeStatus};
use cicada::testing::{TestActors, TokioClock, default_start};
use cicada::{Failed, StopCause, Stopped, Store, WatchTarget};
use serde::{Deserialize, Serialize};

use tokio::sync::Notify;

/// An actor of the one type of these tests: it runs scripts of control effects and other calls on other units.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Unit {
    id: String,
}

impl ActorRef for Unit {
    const TYPE: &'static str = "unit";
    type Message = UnitMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
enum UnitMsg {
    Failed(Failed),
    Stopped(Stopped),
    /// Runs the calls, in order; one that fails fails the turn.
    Run {
        ops: Vec<Op>,
    },
    /// Notes `work`.
    Work,
    /// Notes that it runs, waits for the gate, then notes `waited`.
    Wait,
}

/// One call of a script. `to` is the key of a unit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
enum Op {
    Pause {
        to: String,
    },
    Resume {
        to: String,
    },
    Purge {
        to: String,
    },
    Delete {
        to: String,
    },
    DeleteAt {
        address: String,
    },
    Create {
        to: String,
        n: i64,
    },
    CreateChild {
        to: String,
        n: i64,
    },
    Send {
        to: String,
    },
    Watch {
        to: String,
    },
    Stop,
    Fail,
    /// Runs the call and notes `ok` or `refused: <why>`, without failing the turn.
    Try {
        op: Box<Op>,
    },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Notes {
    #[serde(default)]
    n: i64,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    stopped: Vec<Stopped>,
    /// Deletes the actor of every `Stopped` it handles, by its address.
    #[serde(default)]
    delete_on_stop: bool,
}

/// Lets a test hold a turn while it runs.
#[derive(Default)]
struct Gate {
    running: Notify,
    open: Notify,
}

struct UnitActor(Arc<Gate>);

fn unit(key: &str) -> Unit {
    Unit { id: key.to_owned() }
}

fn address(key: &str) -> ActorAddress {
    unit(key).address().unwrap()
}

fn run(ops: Vec<Op>) -> UnitMsg {
    UnitMsg::Run { ops }
}

fn to(key: &str) -> String {
    key.to_owned()
}

fn attempt(op: Op) -> Op {
    Op::Try { op: Box::new(op) }
}

fn perform(turn: &mut Turn<UnitActor>, op: Op) -> HandlerResult {
    match op {
        Op::Pause { to } => turn.pause(&unit(&to))?,
        Op::Resume { to } => turn.resume(&unit(&to))?,
        Op::Purge { to } => turn.purge(&unit(&to))?,
        Op::Delete { to } => turn.delete(&unit(&to))?,
        Op::DeleteAt { address } => turn.delete_at(&ActorAddress::parse(&address)?)?,
        Op::Create { to, n } => turn.create(&unit(&to), &Notes { n, ..Notes::default() })?,
        Op::CreateChild { to, n } => turn.create_child(&unit(&to), &Notes { n, ..Notes::default() })?,
        Op::Send { to } => {
            turn.send(&unit(&to), UnitMsg::Work)?;
        }
        Op::Watch { to } => turn.watch(&unit(&to))?,
        Op::Stop => turn.stop(),
        Op::Fail => return Err(HandlerError::msg("failed after the calls")),
        Op::Try { op } => {
            let note = match perform(turn, *op) {
                Ok(()) => "ok".to_owned(),
                Err(e) => format!("refused: {e}"),
            };
            turn.state_mut().notes.push(note);
        }
    }
    Ok(())
}

impl Actor for UnitActor {
    type Ref = Unit;
    type State = Notes;

    async fn handle(&self, turn: &mut Turn<Self>, message: UnitMsg) -> HandlerResult {
        match message {
            UnitMsg::Failed(_) => {}
            UnitMsg::Stopped(stopped) => {
                if turn.state().delete_on_stop {
                    turn.delete_at(&stopped.actor)?;
                }
                turn.state_mut().stopped.push(stopped);
            }
            UnitMsg::Run { ops } => {
                for op in ops {
                    perform(turn, op)?;
                }
            }
            UnitMsg::Work => turn.state_mut().notes.push("work".into()),
            UnitMsg::Wait => {
                self.0.running.notify_one();
                self.0.open.notified().await;
                turn.state_mut().notes.push("waited".into());
            }
        }
        Ok(())
    }
}

async fn node() -> (TestActors, Arc<Gate>) {
    let gate = Arc::new(Gate::default());
    let actor = UnitActor(gate.clone());
    let test = TestActors::builder().with(move |n| n.host(actor)).build().await.expect("the node starts");
    (test, gate)
}

/// A node with the units of `keys`, each with `n` 0.
async fn with_units(keys: &[&str]) -> (TestActors, Arc<Gate>) {
    let (test, gate) = node().await;
    for key in keys {
        test.create(&unit(key), &Notes::default()).await;
    }
    (test, gate)
}

async fn notes(test: &TestActors, key: &str) -> Notes {
    test.state(&unit(key)).await
}

async fn status(test: &TestActors, message: &Message) -> Option<OutcomeStatus> {
    test.outcome(message).await.map(|o| o.status)
}

async fn outcome(test: &TestActors, message: &Message) -> OutcomeRecord {
    test.outcome(message).await.expect("finished")
}

async fn paused(test: &TestActors, key: &str) -> bool {
    test.system().get_actor(&address(key)).await.unwrap().expect("exists").paused
}

async fn parent_of(test: &TestActors, key: &str) -> Option<ActorAddress> {
    test.system().get_actor(&address(key)).await.unwrap().expect("exists").parent
}

/// Runs a turn of `parent` that creates `kids` as its children.
async fn spawn(test: &TestActors, parent: &str, kids: &[&str]) {
    let ops = kids.iter().map(|k| Op::CreateChild { to: to(k), n: 0 }).collect();
    let m = test.send(&unit(parent), run(ops)).await;
    test.run_until_idle().await;
    assert_eq!(status(test, &m).await, Some(OutcomeStatus::Done));
}

/// Starts the one due turn, of a `Wait`, and returns once its handler waits at the gate.
async fn hold(test: &TestActors, gate: &Gate) -> tokio::task::JoinHandle<usize> {
    let system = test.system().clone();
    let running = tokio::spawn(async move { system.run_due_turns(1).await.unwrap() });
    gate.running.notified().await;
    running
}

/// Scenario: A pause and a resume.
#[tokio::test(start_paused = true)]
async fn a_message_waits_while_a_turn_paused_its_target_and_is_handled_after_a_turn_resumed_it() {
    let (test, _) = with_units(&["boss", "w"]).await;
    test.send(&unit("boss"), run(vec![Op::Pause { to: to("w") }])).await;
    test.run_until_idle().await;
    assert!(paused(&test, "w").await);
    let work = test.send(&unit("w"), UnitMsg::Work).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &work).await, None, "the message waits");
    test.send(&unit("boss"), run(vec![Op::Resume { to: to("w") }])).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &work).await, Some(OutcomeStatus::Done), "handled after the resume");
}

/// Scenario: A purge.
#[tokio::test(start_paused = true)]
async fn a_purge_by_a_turn_ends_three_pending_messages_and_keeps_the_actor() {
    let (test, _) = with_units(&["boss"]).await;
    test.create(&unit("w"), &Notes { n: 7, ..Notes::default() }).await;
    test.system().pause_mailbox(&address("w")).await.unwrap();
    let pending = [
        test.send(&unit("w"), UnitMsg::Work).await,
        test.send(&unit("w"), UnitMsg::Work).await,
        test.send(&unit("w"), UnitMsg::Work).await,
    ];
    test.send(&unit("boss"), run(vec![Op::Purge { to: to("w") }])).await;
    test.run_until_idle().await;
    for m in &pending {
        assert_eq!(status(&test, m).await, Some(OutcomeStatus::Purged));
    }
    assert_eq!(notes(&test, "w").await.n, 7, "the actor is still there with its state");
}

/// Scenarios: A delete; An actor it did not create.
#[tokio::test(start_paused = true)]
async fn a_delete_by_a_turn_removes_an_actor_created_from_outside_and_purges_its_message() {
    let (test, _) = with_units(&["boss", "w"]).await;
    test.system().pause_mailbox(&address("w")).await.unwrap();
    let waiting = test.send(&unit("w"), UnitMsg::Work).await;
    let m = test.send(&unit("boss"), run(vec![Op::Delete { to: to("w") }])).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(!test.exists(&unit("w")).await, "gone");
    assert_eq!(status(&test, &waiting).await, Some(OutcomeStatus::Purged));
}

/// Scenario: A turn that fails.
#[tokio::test(start_paused = true)]
async fn a_turn_that_fails_applies_none_of_its_control_effects() {
    let (test, _) = with_units(&["boss", "a", "b"]).await;
    let ops = vec![Op::Pause { to: to("a") }, Op::Delete { to: to("b") }, Op::Fail];
    let m = test.send(&unit("boss"), run(ops)).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert!(!paused(&test, "a").await, "not paused");
    assert!(test.exists(&unit("b")).await, "still there");
}

/// Scenario: By address.
#[tokio::test(start_paused = true)]
async fn a_turn_deletes_the_actor_whose_address_it_read_from_a_stopped_message() {
    let (test, _) = with_units(&["w"]).await;
    test.create(&unit("sup"), &Notes { delete_on_stop: true, ..Notes::default() }).await;
    test.send(&unit("sup"), run(vec![Op::Watch { to: to("w") }])).await;
    test.run_until_idle().await;
    // An operator deletes w and creates it again before the supervisor handles its Stopped.
    test.system().delete_actor(&address("w")).await.unwrap();
    test.create(&unit("w"), &Notes::default()).await;
    test.run_until_idle().await;
    assert!(!test.exists(&unit("w")).await, "deleted with delete_at, as with delete and a ref");
    let stopped = notes(&test, "sup").await.stopped;
    assert_eq!(stopped.len(), 2, "{stopped:?}");
    assert_eq!(stopped[0].by, None, "the operator's delete");
    assert_eq!(stopped[1].by, Some(address("sup")), "its own delete, which it is told of as well");
}

/// Scenario: A pause and a purge while a turn runs on the target.
#[tokio::test(start_paused = true)]
async fn a_pause_and_a_purge_let_the_running_turn_commit_and_purge_the_pending_messages() {
    let (test, gate) = with_units(&["boss", "w"]).await;
    let running = test.send(&unit("w"), UnitMsg::Wait).await;
    let held = hold(&test, &gate).await;
    let pending = [test.send(&unit("w"), UnitMsg::Work).await, test.send(&unit("w"), UnitMsg::Work).await];
    test.send(&unit("boss"), run(vec![Op::Pause { to: to("w") }, Op::Purge { to: to("w") }])).await;
    test.run_until_idle().await;
    gate.open.notify_one();
    held.await.unwrap();
    assert_eq!(status(&test, &running).await, Some(OutcomeStatus::Done), "the running turn finishes and commits");
    assert_eq!(notes(&test, "w").await.notes, ["waited"]);
    for m in &pending {
        assert_eq!(status(&test, m).await, Some(OutcomeStatus::Purged));
    }
    let later = test.send(&unit("w"), UnitMsg::Work).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &later).await, None, "no new turn until it is resumed");
}

/// Scenario: A restart.
#[tokio::test(start_paused = true)]
async fn a_restart_gives_the_new_state_purges_the_old_message_and_handles_the_new_one() {
    let (test, _) = with_units(&["boss"]).await;
    test.create(&unit("w"), &Notes { n: 1, ..Notes::default() }).await;
    test.system().pause_mailbox(&address("w")).await.unwrap();
    let old = test.send(&unit("w"), UnitMsg::Work).await;
    let ops = vec![Op::Delete { to: to("w") }, Op::Create { to: to("w"), n: 2 }, Op::Send { to: to("w") }];
    test.send(&unit("boss"), run(ops)).await;
    test.run_until_idle().await;
    let state = notes(&test, "w").await;
    assert_eq!((state.n, state.notes), (2, vec!["work".to_owned()]), "the new state, and the new message handled");
    assert_eq!(status(&test, &old).await, Some(OutcomeStatus::Purged));
    assert!(!paused(&test, "w").await, "a new life");
}

/// Scenario: A parent restarts its child.
#[tokio::test(start_paused = true)]
async fn a_parent_that_restarts_its_child_keeps_it_a_child_and_its_descendants_go() {
    let (test, _) = with_units(&["p"]).await;
    spawn(&test, "p", &["kid"]).await;
    spawn(&test, "kid", &["grandkid"]).await;
    let ops = vec![Op::Delete { to: to("kid") }, Op::CreateChild { to: to("kid"), n: 5 }];
    test.send(&unit("p"), run(ops)).await;
    test.run_until_idle().await;
    assert_eq!(parent_of(&test, "kid").await, Some(address("p")));
    assert_eq!(notes(&test, "kid").await.n, 5);
    assert!(!test.exists(&unit("grandkid")).await);
}

/// Scenario: Another actor restarts a child.
#[tokio::test(start_paused = true)]
async fn a_child_restarted_by_another_actor_has_no_parent_and_the_parent_cannot_take_it_back() {
    let (test, _) = with_units(&["p", "other"]).await;
    spawn(&test, "p", &["kid"]).await;
    let ops = vec![Op::Delete { to: to("kid") }, Op::Create { to: to("kid"), n: 1 }];
    test.send(&unit("other"), run(ops)).await;
    test.run_until_idle().await;
    assert_eq!(parent_of(&test, "kid").await, None, "independent now");
    let m = test.send(&unit("p"), run(vec![Op::CreateChild { to: to("kid"), n: 0 }])).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!(parent_of(&test, "kid").await, None);
}

/// Scenario: A watch and a delete in one turn.
#[tokio::test(start_paused = true)]
async fn a_turn_that_watches_and_deletes_an_actor_is_told_once() {
    let (test, _) = with_units(&["boss", "w"]).await;
    test.send(&unit("boss"), run(vec![Op::Watch { to: to("w") }, Op::Delete { to: to("w") }])).await;
    test.run_until_idle().await;
    let stopped = notes(&test, "boss").await.stopped;
    assert_eq!(stopped, [Stopped { actor: address("w"), cause: StopCause::Deleted, by: Some(address("boss")) }]);
}

/// Scenario: Emptying a mailbox and filling it.
#[tokio::test(start_paused = true)]
async fn a_purge_and_a_send_in_one_turn_empty_the_mailbox_and_put_the_new_message_into_it() {
    let (test, _) = with_units(&["boss", "w"]).await;
    test.system().pause_mailbox(&address("w")).await.unwrap();
    let before = [test.send(&unit("w"), UnitMsg::Work).await, test.send(&unit("w"), UnitMsg::Work).await];
    let ops = vec![Op::Send { to: to("w") }, Op::Purge { to: to("w") }, Op::Resume { to: to("w") }];
    test.send(&unit("boss"), run(ops)).await;
    test.run_until_idle().await;
    for m in &before {
        assert_eq!(status(&test, m).await, Some(OutcomeStatus::Purged));
    }
    assert_eq!(notes(&test, "w").await.notes, ["work"], "the new message is handled");
}

/// Scenario: Creating an actor that starts paused.
#[tokio::test(start_paused = true)]
async fn an_actor_created_paused_with_a_message_handles_it_once_resumed() {
    let (test, _) = with_units(&["boss"]).await;
    let ops = vec![Op::Send { to: to("w") }, Op::Pause { to: to("w") }, Op::Create { to: to("w"), n: 0 }];
    let m = test.send(&unit("boss"), run(ops)).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(paused(&test, "w").await);
    assert!(notes(&test, "w").await.notes.is_empty(), "the message waits");
    test.system().resume_mailbox(&address("w")).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(notes(&test, "w").await.notes, ["work"]);
}

/// Scenario: The target stopped itself.
#[tokio::test(start_paused = true)]
async fn control_effects_on_an_actor_that_stopped_itself_do_nothing_and_the_turn_commits() {
    let (test, _) = with_units(&["boss", "w", "x"]).await;
    test.send(&unit("w"), run(vec![Op::Stop])).await;
    test.run_until_idle().await;
    let ops = vec![
        Op::Pause { to: to("w") },
        Op::Purge { to: to("w") },
        Op::Delete { to: to("w") },
        Op::Send { to: to("x") },
    ];
    let m = test.send(&unit("boss"), run(ops)).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "the commit succeeds");
    assert_eq!(notes(&test, "x").await.notes, ["work"], "the other effects are applied");
    assert!(!test.exists(&unit("w")).await, "nothing is created");
}

/// Scenarios: A commit that is rejected removes nothing; A send still needs its target.
#[tokio::test(start_paused = true)]
async fn a_send_to_an_absent_or_deleted_actor_rejects_the_commit_and_removes_nothing() {
    let (test, _) = with_units(&["boss", "a", "b"]).await;
    let to_absent =
        test.send(&unit("boss"), run(vec![Op::Delete { to: to("a") }, Op::Send { to: to("nobody") }])).await;
    let to_deleted = test.send(&unit("boss"), run(vec![Op::Delete { to: to("b") }, Op::Send { to: to("b") }])).await;
    test.run_until_idle().await;
    for m in [&to_absent, &to_deleted] {
        assert_eq!(status(&test, m).await, Some(OutcomeStatus::Failed));
    }
    assert!(test.exists(&unit("a")).await && test.exists(&unit("b")).await, "nothing was removed");
}

/// Scenarios: A turn is running on the target; A restart while a turn is running on the target.
#[tokio::test(start_paused = true)]
async fn a_delete_or_a_restart_voids_the_turn_running_on_the_target() {
    for restart in [false, true] {
        let (test, gate) = with_units(&["boss", "w"]).await;
        let running = test.send(&unit("w"), UnitMsg::Wait).await;
        let held = hold(&test, &gate).await;
        let mut ops = vec![Op::Delete { to: to("w") }];
        if restart {
            ops.push(Op::Create { to: to("w"), n: 9 });
        }
        test.send(&unit("boss"), run(ops)).await;
        test.run_until_idle().await;
        gate.open.notify_one();
        held.await.unwrap();
        assert_eq!(status(&test, &running).await, Some(OutcomeStatus::Purged), "restart: {restart}");
        if restart {
            let state = notes(&test, "w").await;
            assert_eq!((state.n, state.notes.len()), (9, 0), "the state of the create, not of the old handler");
            let work = test.send(&unit("w"), UnitMsg::Work).await;
            test.run_until_idle().await;
            assert_eq!(status(&test, &work).await, Some(OutcomeStatus::Done), "the new actor can be claimed");
        } else {
            assert!(!test.exists(&unit("w")).await, "the old turn committed nothing");
        }
    }
}

/// Scenarios: The target has children; The target is watched; An actor is deleted by a turn (watches).
#[tokio::test(start_paused = true)]
async fn a_delete_by_a_turn_removes_the_children_and_tells_the_watchers_who_deleted_them() {
    let (test, _) = with_units(&["boss", "w", "sup"]).await;
    spawn(&test, "w", &["kid"]).await;
    test.send(&unit("sup"), run(vec![Op::Watch { to: to("w") }, Op::Watch { to: to("kid") }])).await;
    test.run_until_idle().await;
    let m = test.send(&unit("boss"), run(vec![Op::Delete { to: to("w") }])).await;
    test.run_until_idle().await;
    assert!(!test.exists(&unit("w")).await && !test.exists(&unit("kid")).await, "the child is gone too");
    let mut stopped = notes(&test, "sup").await.stopped;
    stopped.sort_by(|a, b| a.actor.cmp(&b.actor));
    assert_eq!(
        stopped,
        [
            Stopped { actor: address("kid"), cause: StopCause::Parent, by: Some(address("boss")) },
            Stopped { actor: address("w"), cause: StopCause::Deleted, by: Some(address("boss")) },
        ]
    );
    let told = test.system().outcomes(None, "unit/sup", None, None, 100).await.unwrap().items;
    let notifications: Vec<_> = told.iter().filter(|o| o.envelope.notification).collect();
    assert_eq!(notifications.len(), 2);
    assert!(notifications.iter().all(|o| o.envelope.caused_by.as_ref() == Some(&m.id)), "caused by the turn's message");
}

/// Scenarios: Deleting itself; Pausing itself.
#[tokio::test(start_paused = true)]
async fn a_turn_cannot_delete_its_own_actor_but_can_pause_it_and_stop() {
    let (test, _) = with_units(&["me", "pausing"]).await;
    let m = test.send(&unit("me"), run(vec![attempt(Op::Delete { to: to("me") })])).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "the turn goes on");
    let refused = &notes(&test, "me").await.notes[0];
    assert!(refused.starts_with("refused: ") && refused.contains("turn.stop()"), "{refused}");
    let m = test.send(&unit("me"), run(vec![attempt(Op::Delete { to: to("me") }), Op::Stop])).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(!test.exists(&unit("me")).await, "it stopped itself");

    let m = test.send(&unit("pausing"), run(vec![Op::Pause { to: to("pausing") }])).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    let next = test.send(&unit("pausing"), UnitMsg::Work).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &next).await, None, "the next message waits until it is resumed");
    test.system().resume_mailbox(&address("pausing")).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(status(&test, &next).await, Some(OutcomeStatus::Done));
}

/// Scenario: Deleting its parent.
#[tokio::test(start_paused = true)]
async fn a_turn_that_deletes_its_parent_fails_and_removes_nothing() {
    let (test, _) = with_units(&["p"]).await;
    spawn(&test, "p", &["kid"]).await;
    let m = test.send(&unit("kid"), run(vec![Op::Delete { to: to("p") }])).await;
    test.run_until_idle().await;
    let failed = outcome(&test, &m).await;
    assert_eq!(failed.status, OutcomeStatus::Failed);
    assert!(failed.error.as_deref().unwrap().contains("unit/p"), "{failed:?}");
    assert!(test.exists(&unit("p")).await && test.exists(&unit("kid")).await);
}

/// Scenarios: Pause and resume; The same effect twice; A delete after a create.
#[tokio::test(start_paused = true)]
async fn calls_of_one_turn_on_one_actor_have_one_result() {
    let (test, _) = with_units(&["boss", "a", "b"]).await;
    let ops = vec![
        attempt(Op::Pause { to: to("a") }),
        attempt(Op::Resume { to: to("a") }),
        attempt(Op::Delete { to: to("b") }),
        attempt(Op::Delete { to: to("b") }),
        attempt(Op::Create { to: to("c"), n: 0 }),
        attempt(Op::Delete { to: to("c") }),
    ];
    let m = test.send(&unit("boss"), run(ops)).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    let calls = notes(&test, "boss").await.notes;
    assert_eq!(&calls[..5], ["ok", "ok", "ok", "ok", "ok"]);
    assert!(
        calls[5].starts_with("refused: ") && calls[5].contains("a restart is a delete, then a create"),
        "{calls:?}"
    );
    assert!(!paused(&test, "a").await, "the last call stands");
    assert!(!test.exists(&unit("b")).await, "deleted once");
    assert!(test.exists(&unit("c")).await, "the create stands");
}

/// Scenario: A create of a descendant of a deleted actor.
#[tokio::test(start_paused = true)]
async fn a_create_of_a_descendant_of_a_deleted_actor_makes_a_new_one_in_either_order() {
    for create_first in [true, false] {
        let (test, _) = with_units(&["boss", "x"]).await;
        spawn(&test, "x", &["y"]).await;
        let (create, delete) = (Op::Create { to: to("y"), n: 3 }, Op::Delete { to: to("x") });
        let ops = if create_first { vec![create, delete] } else { vec![delete, create] };
        let m = test.send(&unit("boss"), run(ops)).await;
        test.run_until_idle().await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "create first: {create_first}");
        assert!(!test.exists(&unit("x")).await);
        assert_eq!(notes(&test, "y").await.n, 3, "a new y, created first: {create_first}");
        assert_eq!(parent_of(&test, "y").await, None);
    }
}

/// A started node on the in-memory store with a long poll interval, so that only a wake-up runs a turn soon.
async fn started() -> (ActorSystem, Store) {
    let store = Store::memory_on(Arc::new(TokioClock::new(default_start())));
    let config = NodeConfig { poll_interval: Duration::from_secs(3600), ..NodeConfig::default() };
    let system = ActorSystem::builder(store.clone()).config(config).host(UnitActor(Arc::default())).build();
    system.start().await.unwrap();
    (system, store)
}

async fn state_of(system: &ActorSystem, key: &str) -> Notes {
    let snapshot = system.get_actor(&address(key)).await.unwrap().expect("exists");
    serde_json::from_value(snapshot.state).unwrap()
}

/// Scenario: A resume wakes the node.
#[tokio::test(start_paused = true)]
async fn a_resume_by_a_turn_wakes_the_node_of_the_target() {
    let (system, store) = started().await;
    for key in ["boss", "w"] {
        system.create(&unit(key), &Notes::default()).await.unwrap();
    }
    system.pause_mailbox(&address("w")).await.unwrap();
    system.send(&unit("w"), UnitMsg::Work).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await; // the dispatch loop sleeps for its poll interval now
    system.send(&unit("boss"), run(vec![Op::Resume { to: to("w") }])).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(state_of(&system, "w").await.notes, ["work"], "handled at once, not after the next poll");
    system.stop().await;
    store.close().await;
}

/// Scenario: A delete by a turn wakes the watcher.
#[tokio::test(start_paused = true)]
async fn a_delete_by_a_turn_wakes_the_node_of_the_watcher() {
    let (system, store) = started().await;
    for key in ["boss", "w", "sup"] {
        system.create(&unit(key), &Notes::default()).await.unwrap();
    }
    system.watch(&WatchTarget::Actor(address("w")), &address("sup")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await; // the dispatch loop sleeps for its poll interval now
    system.send(&unit("boss"), run(vec![Op::Delete { to: to("w") }])).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(state_of(&system, "sup").await.stopped.len(), 1, "handled at once, not after the next poll");
    system.stop().await;
    store.close().await;
}

/// Scenario: A resume wakes the node, for a turn that a started node runs by hand (`run_due_turns`). Its dispatch
/// loop did not run the turn, so only the wake-up after the commit tells it.
#[tokio::test(start_paused = true)]
async fn a_resume_by_a_turn_that_a_node_runs_by_hand_wakes_its_dispatch_loop() {
    let (system, store) = started().await;
    let elsewhere = ActorSystem::builder(store.clone()).host(UnitActor(Arc::default())).build();
    for key in ["boss", "w"] {
        system.create(&unit(key), &Notes::default()).await.unwrap();
    }
    system.pause_mailbox(&address("w")).await.unwrap();
    system.send(&unit("w"), UnitMsg::Work).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await; // the dispatch loop sleeps for its poll interval now
    elsewhere.send(&unit("boss"), run(vec![Op::Resume { to: to("w") }])).await.unwrap(); // does not wake `system`
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(state_of(&system, "boss").await.notes.is_empty());
    assert_eq!(system.run_due_turns(1).await.unwrap(), 1, "the turn of boss, run by hand");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(state_of(&system, "w").await.notes, ["work"], "handled at once, not after the next poll");
    system.stop().await;
    store.close().await;
}
