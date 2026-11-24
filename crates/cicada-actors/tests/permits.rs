//! The permit pool, on virtual time with `TestActors` (ADR 0058): grants in FIFO order with fenced numbers, answers in
//! the commit of each `Acquire`, a full queue, releases and cancels, resizes, audits, the undelivered grant that comes
//! back, and the refusals for good.

mod common;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::{FailureCause, WatchTarget};
use cicada_actors::{PermitNotice, PermitNoticeKind, Permits, PermitsActor, PermitsMsg, PermitsState};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};

/// `holder/{id}`: keeps every notice of a pool it is told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Holder {
    id: String,
}

impl ActorRef for Holder {
    const TYPE: &'static str = "holder";
    type Message = HolderMsg;
}

fn holder(id: &str) -> Holder {
    Holder { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum HolderMsg {
    Permit(PermitNotice),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Notices {
    #[serde(default)]
    log: Vec<PermitNotice>,
}

struct HolderActor;

impl Actor for HolderActor {
    type Ref = Holder;
    type State = Notices;

    async fn handle(&self, turn: &mut Turn<Self>, message: HolderMsg) -> HandlerResult {
        let HolderMsg::Permit(notice) = message;
        turn.state_mut().log.push(notice);
        Ok(())
    }
}

fn pool() -> Permits {
    Permits::named("builds")
}

/// A node with the pool `cicada.permits/builds` of `capacity` and `max_waiting`, the holders `holder/a` through
/// `holder/e`, the worker `worker/a`, and the witness, which watches the pool.
async fn node(capacity: u32, max_waiting: u32) -> TestActors {
    let test = TestActors::builder()
        .with(|node| node.host(PermitsActor).host(HolderActor).host(WorkerActor).host(WitnessActor))
        .build()
        .await
        .unwrap();
    test.create(&pool(), &PermitsState::new(capacity, max_waiting).unwrap()).await;
    for id in ["a", "b", "c", "d", "e"] {
        test.create(&holder(id), &Notices::default()).await;
    }
    test.create(&worker("a"), &WorkerState::default()).await;
    witness(&test, "cicada.permits").await;
    test
}

/// The `Acquire` of request `id` by `holder/{id}`.
fn acquire(id: &str) -> PermitsMsg {
    PermitsMsg::acquire(id, &holder(id), "Permit").unwrap()
}

async fn ask(test: &TestActors, id: &str) {
    test.send(&pool(), acquire(id)).await;
    test.run_until_idle().await;
}

async fn release(test: &TestActors, request: &str, grant: u64) {
    test.send(&pool(), PermitsMsg::Release { request: request.into(), grant }).await;
    test.run_until_idle().await;
}

async fn state(test: &TestActors) -> PermitsState {
    test.state::<_, PermitsState>(&pool()).await
}

/// The requests that hold a permit, with their grant numbers.
async fn holders(test: &TestActors) -> Vec<(String, u64)> {
    state(test).await.holders.into_iter().map(|h| (h.request, h.grant)).collect()
}

/// The waiting requests, in order.
async fn waiting(test: &TestActors) -> Vec<String> {
    state(test).await.waiting.into_iter().map(|w| w.request).collect()
}

/// The notices `holder/{id}` got: their kind and grant.
async fn notices(test: &TestActors, id: &str) -> Vec<(PermitNoticeKind, Option<u64>)> {
    let log = test.state::<_, Notices>(&holder(id)).await.log;
    log.into_iter().map(|n| (n.kind, n.grant)).collect()
}

/// The `Failed` the witness was told of the `Notify` messages of the pool.
async fn failed_notifies(test: &TestActors) -> Vec<cicada::Failed> {
    told(test).await.into_iter().filter(|f| f.message_type == "Notify").collect()
}

use PermitNoticeKind::{Check, Granted, Queued, Refused};

/// Scenario: A pool from its create.
#[tokio::test(start_paused = true)]
async fn a_pool_from_its_create_has_its_bounds_and_nothing_in_its_books() {
    let test = node(2, 10).await;
    assert_eq!(state(&test).await, PermitsState::new(2, 10).unwrap());
    let json = test.state_json(&pool()).await.unwrap();
    assert_eq!(
        json,
        serde_json::json!({"capacity": 2, "maxWaiting": 10, "nextGrant": 1, "holders": [],
        "waiting": []})
    );
}

/// An `Acquire` with room is granted in its own commit, and the notice names the pool and the request.
#[tokio::test(start_paused = true)]
async fn an_acquire_with_room_is_granted_at_once() {
    let test = node(2, 10).await;
    ask(&test, "a").await;
    assert_eq!(holders(&test).await, [("a".to_owned(), 1)]);
    assert!(state(&test).await.holders[0].notified);
    let log = test.state::<_, Notices>(&holder("a")).await.log;
    let granted = PermitNotice { pool: "builds".into(), request: "a".into(), kind: Granted, grant: Some(1) };
    assert_eq!(log, [granted]);
    assert_eq!(state(&test).await.next_grant, 2);
}

/// Scenario: FIFO order.
#[tokio::test(start_paused = true)]
async fn the_head_of_the_queue_is_granted_in_the_commit_that_made_room_and_told_in_a_later_turn() {
    let test = node(1, 10).await;
    for id in ["e", "a", "b", "c"] {
        ask(&test, id).await;
    }
    assert_eq!(waiting(&test).await, ["a", "b", "c"]);
    test.send(&pool(), PermitsMsg::Release { request: "e".into(), grant: 1 }).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(holders(&test).await, [("a".to_owned(), 2)], "the commit of the release grants a");
    assert!(!state(&test).await.holders[0].notified);
    assert_eq!(waiting(&test).await, ["b", "c"]);
    assert_eq!(notices(&test, "a").await, [(Queued, None)], "not told yet");
    let pending = test.pending(&pool()).await;
    assert_eq!(pending.iter().map(|m| m.message_type.as_str()).collect::<Vec<_>>(), ["Notify"]);
    test.run_until_idle().await;
    assert_eq!(notices(&test, "a").await, [(Queued, None), (Granted, Some(2))]);
    assert!(state(&test).await.holders[0].notified);
    assert_eq!(notices(&test, "b").await, [(Queued, None)]);
    assert_eq!(waiting(&test).await, ["b", "c"]);
}

/// Scenario: Grant numbers only go up.
#[tokio::test(start_paused = true)]
async fn a_later_grant_has_a_higher_number() {
    let test = node(1, 10).await;
    ask(&test, "a").await;
    release(&test, "a", 1).await;
    ask(&test, "b").await;
    assert_eq!(holders(&test).await, [("b".to_owned(), 2)]);
    assert_eq!(notices(&test, "b").await, [(Granted, Some(2))]);
}

/// Scenario: A repeated acquire of a holder. The repeat names another target: the first one is kept.
#[tokio::test(start_paused = true)]
async fn a_repeated_acquire_of_a_holder_answers_the_same_grant_to_the_first_target() {
    let test = node(2, 10).await;
    ask(&test, "a").await;
    test.send(&pool(), PermitsMsg::acquire("a", &holder("b"), "Permit").unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(notices(&test, "a").await, [(Granted, Some(1)), (Granted, Some(1))]);
    assert!(notices(&test, "b").await.is_empty());
    assert_eq!(holders(&test).await, [("a".to_owned(), 1)], "one permit for it");
}

/// Scenario: A repeated acquire of a waiter.
#[tokio::test(start_paused = true)]
async fn a_repeated_acquire_of_a_waiter_answers_queued_and_keeps_its_place() {
    let test = node(1, 10).await;
    for id in ["e", "a", "b", "c"] {
        ask(&test, id).await;
    }
    test.send(&pool(), PermitsMsg::acquire("b", &holder("d"), "Permit").unwrap()).await;
    test.run_until_idle().await;
    assert_eq!(notices(&test, "b").await, [(Queued, None), (Queued, None)]);
    assert!(notices(&test, "d").await.is_empty(), "the first target is kept");
    assert_eq!(waiting(&test).await, ["a", "b", "c"]);
}

/// Scenario: An acquire with a gone target, with room and with a full pool.
#[tokio::test(start_paused = true)]
async fn an_acquire_with_a_gone_target_is_rejected_and_records_nothing() {
    let test = node(1, 10).await;
    for ghost in ["ghost-1", "ghost-2"] {
        let version = test.version(&pool()).await;
        let m = test.send(&pool(), PermitsMsg::acquire(ghost, &holder(ghost), "Permit").unwrap()).await;
        test.run_until_idle().await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
        let failed = told(&test).await;
        assert_eq!(failed.last().map(|f| f.cause), Some(FailureCause::Commit));
        assert_eq!(test.version(&pool()).await, version, "nothing recorded");
        ask(&test, "a").await;
    }
    assert_eq!(holders(&test).await, [("a".to_owned(), 1)]);
    assert!(waiting(&test).await.is_empty());
}

/// Scenario: A full queue.
#[tokio::test(start_paused = true)]
async fn a_full_queue_refuses_and_records_nothing() {
    let test = node(1, 1).await;
    ask(&test, "a").await;
    ask(&test, "b").await;
    let version = test.version(&pool()).await;
    ask(&test, "c").await;
    assert_eq!(notices(&test, "c").await, [(Refused, None)]);
    assert_eq!(test.version(&pool()).await, version);
    assert_eq!(waiting(&test).await, ["b"]);
}

/// A capacity of 0 pauses the pool: every request waits.
#[tokio::test(start_paused = true)]
async fn a_pool_of_capacity_0_grants_nothing() {
    let test = node(0, 10).await;
    ask(&test, "a").await;
    assert_eq!(notices(&test, "a").await, [(Queued, None)]);
    assert!(holders(&test).await.is_empty());
}

/// Sends `message` to the pool and checks that it is refused for good, with the state and the holders unchanged.
async fn assert_refused(test: &TestActors, message: PermitsMsg) -> cicada::Failed {
    let before = state(test).await;
    let version = test.version(&pool()).await;
    let m = test.send(&pool(), message).await;
    test.run_until_idle().await;
    assert_eq!(status(test, &m).await, Some(OutcomeStatus::Failed));
    let failed = told(test).await.into_iter().find(|f| f.message_id == m.id).expect("a Failed");
    assert_eq!(failed.cause, FailureCause::Refused, "{}", failed.error);
    assert_eq!(failed.attempts, 1, "a refusal is not tried again");
    assert_eq!(test.version(&pool()).await, version);
    assert_eq!(state(test).await, before);
    failed
}

/// Scenario: A resize above the bound.
#[tokio::test(start_paused = true)]
async fn a_resize_above_1000_is_refused_for_good() {
    let test = node(2, 10).await;
    let failed = assert_refused(&test, PermitsMsg::Resize { capacity: 1001 }).await;
    assert!(failed.error.contains("capacity must be 0 through 1000"), "{}", failed.error);
    assert_eq!(state(&test).await.capacity, 2);
}

/// Scenario: A bad request ID. Also an empty one, and in a cancel; a NUL is refused at the send.
#[tokio::test(start_paused = true)]
async fn an_acquire_with_a_bad_request_id_is_refused_for_good_and_sends_nothing() {
    let test = node(2, 10).await;
    for request in ["r".repeat(201), String::new()] {
        let message = PermitsMsg::Acquire { request, to: addr(&holder("a")), message_type: "Permit".into() };
        let failed = assert_refused(&test, message).await;
        assert!(failed.error.contains("request ID"), "{}", failed.error);
    }
    assert!(notices(&test, "a").await.is_empty());
    assert_refused(&test, PermitsMsg::Cancel { request: String::new() }).await;
}

/// Scenario: A message type the target does not take. Also a target without a registered type, and a message type
/// out of bounds.
#[tokio::test(start_paused = true)]
async fn an_acquire_whose_notice_the_send_refuses_is_refused_for_good_and_does_not_queue() {
    let test = node(1, 10).await;
    ask(&test, "a").await;
    let wrong = PermitsMsg::Acquire { request: "w".into(), to: addr(&worker("a")), message_type: "Permit".into() };
    let failed = assert_refused(&test, wrong).await;
    assert!(failed.error.contains("Permit"), "{}", failed.error);
    let nobody = PermitsMsg::Acquire {
        request: "n".into(),
        to: ActorAddress::parse("nobody/x").unwrap(),
        message_type: "Permit".into(),
    };
    assert_refused(&test, nobody).await;
    let long = PermitsMsg::Acquire { request: "l".into(), to: addr(&holder("b")), message_type: "P".repeat(201) };
    let failed = assert_refused(&test, long).await;
    assert!(failed.error.contains("messageType"), "{}", failed.error);
    assert!(waiting(&test).await.is_empty());
}

/// Scenario: A bad request ID in a release.
#[tokio::test(start_paused = true)]
async fn a_release_with_an_empty_request_id_is_refused_for_good() {
    let test = node(1, 10).await;
    ask(&test, "a").await;
    assert_refused(&test, PermitsMsg::Release { request: String::new(), grant: 1 }).await;
    assert_eq!(holders(&test).await, [("a".to_owned(), 1)]);
}

/// Scenario: A stale release. The request is granted, released, and granted again under the same ID.
#[tokio::test(start_paused = true)]
async fn a_release_of_an_earlier_grant_is_ignored() {
    let test = node(1, 10).await;
    ask(&test, "a").await;
    release(&test, "a", 1).await;
    ask(&test, "a").await;
    assert_eq!(holders(&test).await, [("a".to_owned(), 2)], "a reused ID is a new request");
    let version = test.version(&pool()).await;
    release(&test, "a", 1).await;
    assert_eq!(holders(&test).await, [("a".to_owned(), 2)]);
    assert_eq!(test.version(&pool()).await, version, "nothing changes");
    release(&test, "b", 2).await;
    assert_eq!(holders(&test).await, [("a".to_owned(), 2)], "another request's release frees nothing");
}

/// Scenario: Cancel while waiting.
#[tokio::test(start_paused = true)]
async fn a_cancelled_waiter_leaves_the_queue_and_is_never_granted() {
    let test = node(1, 10).await;
    for id in ["e", "a", "b", "c"] {
        ask(&test, id).await;
    }
    test.send(&pool(), PermitsMsg::Cancel { request: "b".into() }).await;
    test.run_until_idle().await;
    assert_eq!(waiting(&test).await, ["a", "c"]);
    release(&test, "e", 1).await;
    release(&test, "a", 2).await;
    assert_eq!(holders(&test).await, [("c".to_owned(), 3)]);
    assert_eq!(notices(&test, "b").await, [(Queued, None)]);
}

/// Scenario: Cancel of a holder.
#[tokio::test(start_paused = true)]
async fn a_cancel_of_a_holder_keeps_its_permit() {
    let test = node(1, 10).await;
    ask(&test, "a").await;
    ask(&test, "b").await;
    test.send(&pool(), PermitsMsg::Cancel { request: "a".into() }).await;
    test.run_until_idle().await;
    assert_eq!(holders(&test).await, [("a".to_owned(), 1)]);
    assert_eq!(waiting(&test).await, ["b"]);
}

/// Scenario: A deleted waiter.
#[tokio::test(start_paused = true)]
async fn a_grant_to_a_deleted_waiter_comes_back_and_goes_to_the_next() {
    let test = node(1, 10).await;
    for id in ["e", "a", "b"] {
        ask(&test, id).await;
    }
    test.system().delete_actor(&addr(&holder("a"))).await.unwrap();
    let m = test.send(&pool(), PermitsMsg::Release { request: "e".into(), grant: 1 }).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "the release commits");
    let failed = failed_notifies(&test).await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!((failed[0].cause, &failed[0].payload["request"]), (FailureCause::Commit, &serde_json::json!("a")));
    assert_eq!(holders(&test).await, [("b".to_owned(), 3)]);
    assert!(waiting(&test).await.is_empty());
    assert_eq!(notices(&test, "b").await, [(Queued, None), (Granted, Some(3))]);
}

/// Scenario: A failed check.
#[tokio::test(start_paused = true)]
async fn a_failed_check_keeps_the_permit() {
    let test = node(2, 10).await;
    ask(&test, "a").await;
    test.system().delete_actor(&addr(&holder("a"))).await.unwrap();
    test.send(&pool(), PermitsMsg::Audit).await;
    test.run_until_idle().await;
    let failed = failed_notifies(&test).await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].payload["check"], true);
    assert_eq!(holders(&test).await, [("a".to_owned(), 1)]);
}

/// Scenario: A message the pool did not send itself: a `Notify` for a waiter and for a holder.
#[tokio::test(start_paused = true)]
async fn a_notify_sent_by_a_client_sends_nothing() {
    let test = node(1, 10).await;
    ask(&test, "a").await;
    ask(&test, "b").await;
    let version = test.version(&pool()).await;
    for (request, check) in [("b", false), ("b", true), ("a", false), ("a", true)] {
        let m = test.send(&pool(), PermitsMsg::Notify { request: request.into(), check }).await;
        test.run_until_idle().await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    }
    assert_eq!(notices(&test, "a").await, [(Granted, Some(1))]);
    assert_eq!(notices(&test, "b").await, [(Queued, None)]);
    assert_eq!(test.version(&pool()).await, version);
}

/// Scenario: A message the pool did not send itself: a `Failed` that names a grant notice of a holder never told.
#[tokio::test(start_paused = true)]
async fn a_failed_sent_by_a_client_frees_nothing() {
    let test = node(1, 10).await;
    test.system().delete_actor(&addr(&pool())).await.unwrap();
    let untold = serde_json::json!({"capacity": 1, "maxWaiting": 10, "nextGrant": 2, "holders": [
        {"request": "a", "to": "holder/a", "messageType": "Permit", "grant": 1, "notified": false}]});
    test.create(&pool(), &untold).await;
    let notify = cicada::message::encode(&PermitsMsg::Notify { request: "a".into(), check: false }).unwrap();
    let planted = cicada::Failed {
        actor: addr(&pool()),
        message_id: MessageId::new("m-planted").unwrap(),
        message_type: notify.message_type,
        payload: notify.payload,
        correlation_id: None,
        from: Some(addr(&pool())),
        error: "made up".into(),
        attempts: 1,
        cause: FailureCause::Commit,
    };
    let version = test.version(&pool()).await;
    let m = test.send(&pool(), PermitsMsg::Failed(planted)).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(holders(&test).await, [("a".to_owned(), 1)]);
    assert_eq!(test.version(&pool()).await, version);
}

/// Scenario: A resize up.
#[tokio::test(start_paused = true)]
async fn a_resize_up_grants_the_waiters_in_its_commit() {
    let test = node(1, 10).await;
    for id in ["e", "a", "b"] {
        ask(&test, id).await;
    }
    test.send(&pool(), PermitsMsg::Resize { capacity: 3 }).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(holders(&test).await, [("e".to_owned(), 1), ("a".to_owned(), 2), ("b".to_owned(), 3)]);
    assert!(waiting(&test).await.is_empty());
    assert_eq!(notices(&test, "a").await, [(Queued, None)], "told in later turns");
    test.run_until_idle().await;
    assert_eq!(notices(&test, "a").await, [(Queued, None), (Granted, Some(2))]);
    assert_eq!(notices(&test, "b").await, [(Queued, None), (Granted, Some(3))]);
}

/// Scenario: A resize down.
#[tokio::test(start_paused = true)]
async fn a_resize_down_revokes_nothing_and_holds_new_grants_back() {
    let test = node(3, 10).await;
    for id in ["a", "b", "c", "d"] {
        ask(&test, id).await;
    }
    test.send(&pool(), PermitsMsg::Resize { capacity: 1 }).await;
    test.run_until_idle().await;
    assert_eq!(holders(&test).await.len(), 3, "nothing is revoked");
    release(&test, "a", 1).await;
    assert_eq!(holders(&test).await, [("b".to_owned(), 2), ("c".to_owned(), 3)]);
    assert_eq!(waiting(&test).await, ["d"], "not granted while two hold");
    release(&test, "b", 2).await;
    assert_eq!(waiting(&test).await, ["d"], "not granted while one holds");
    release(&test, "c", 3).await;
    assert_eq!(holders(&test).await, [("d".to_owned(), 4)]);
}

/// Scenario: An audit frees only on a release. A waiting request gets a `check` without a number.
#[tokio::test(start_paused = true)]
async fn an_audit_asks_every_request_and_frees_nothing_itself() {
    let test = node(2, 10).await;
    for id in ["a", "b", "c"] {
        ask(&test, id).await;
    }
    test.send(&pool(), PermitsMsg::Audit).await;
    test.run_until_idle().await;
    assert_eq!(notices(&test, "a").await, [(Granted, Some(1)), (Check, Some(1))]);
    assert_eq!(notices(&test, "b").await, [(Granted, Some(2)), (Check, Some(2))]);
    assert_eq!(notices(&test, "c").await, [(Queued, None), (Check, None)]);
    assert_eq!(holders(&test).await.len(), 2, "the audit frees nothing");
    release(&test, "a", 1).await;
    assert_eq!(holders(&test).await, [("b".to_owned(), 2), ("c".to_owned(), 3)]);
}

/// An audit sends no `check` to a holder whose `granted` notice has not committed; once it has, the next audit does.
#[tokio::test(start_paused = true)]
async fn an_audit_checks_a_holder_only_after_its_grant_notice_committed() {
    let test = node(1, 10).await;
    test.system().delete_actor(&addr(&pool())).await.unwrap();
    let untold = serde_json::json!({"capacity": 1, "maxWaiting": 10, "nextGrant": 2, "holders": [
        {"request": "a", "to": "holder/a", "messageType": "Permit", "grant": 1, "notified": false}]});
    test.create(&pool(), &untold).await;
    test.send(&pool(), PermitsMsg::Audit).await;
    test.run_until_idle().await;
    assert!(notices(&test, "a").await.is_empty(), "no check before its grant notice");
    ask(&test, "a").await;
    assert_eq!(notices(&test, "a").await, [(Granted, Some(1))]);
    assert!(state(&test).await.holders[0].notified);
    test.send(&pool(), PermitsMsg::Audit).await;
    test.run_until_idle().await;
    assert_eq!(notices(&test, "a").await, [(Granted, Some(1)), (Check, Some(1))]);
}

/// A waiter checked by an audit and granted before its `check` runs gets no `check`: its number comes by `granted`.
#[tokio::test(start_paused = true)]
async fn a_waiter_granted_before_its_check_runs_gets_only_its_grant() {
    let test = node(1, 10).await;
    ask(&test, "e").await;
    ask(&test, "a").await;
    test.send(&pool(), PermitsMsg::Audit).await;
    test.send(&pool(), PermitsMsg::Release { request: "e".into(), grant: 1 }).await;
    test.run_until_idle().await;
    assert_eq!(notices(&test, "a").await, [(Queued, None), (Granted, Some(2))]);
    assert_eq!(notices(&test, "e").await, [(Granted, Some(1))], "a released holder is no longer checked");
    assert_eq!(holders(&test).await, [("a".to_owned(), 2)]);
}

/// A notice of a `Notify` carries the correlation id of the message that made it.
#[tokio::test(start_paused = true)]
async fn a_grant_notice_carries_the_correlation_id_of_the_release() {
    let test = node(1, 10).await;
    ask(&test, "a").await;
    ask(&test, "b").await;
    test.system().pause_mailbox(&addr(&holder("b"))).await.unwrap();
    let release = PermitsMsg::Release { request: "a".into(), grant: 1 };
    test.send_with(&pool(), release, SendOptions::new().correlation_id("c1")).await;
    test.run_until_idle().await;
    let pending = test.pending(&holder("b")).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].correlation_id.as_deref(), Some("c1"));
    assert!(pending[0].retry.is_some(), "with the shared retry policy");
}

async fn pool_watches(test: &TestActors) -> Vec<cicada::watch::Watch> {
    test.system().watches(Some(&addr(&pool())), None, None, 10).await.unwrap().items
}

/// The pool watches only itself, and only once it sent a `Notify`.
#[tokio::test(start_paused = true)]
async fn a_pool_watches_itself_once_it_sends_a_notify() {
    let test = node(1, 10).await;
    ask(&test, "a").await;
    assert!(pool_watches(&test).await.is_empty(), "an answer in the commit needs no watch");
    test.send(&pool(), PermitsMsg::Audit).await;
    test.run_until_idle().await;
    let targets: Vec<String> = pool_watches(&test).await.iter().map(|w| w.target.to_string()).collect();
    assert_eq!(targets, ["cicada.permits/builds"]);
    let of_holder = test.system().watches(None, Some(&WatchTarget::from(addr(&holder("a")))), None, 10).await;
    assert!(of_holder.unwrap().items.is_empty());
}
