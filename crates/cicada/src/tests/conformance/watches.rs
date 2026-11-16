//! Watches (ADR 0018, ADR 0023): a commit and a call from outside set and remove watches; a commit that ends a message
//! `failed`, a stop and a delete write one notification per watcher and actor, named after the event, marked, in a
//! fixed order; watches follow their watcher; a notification that fails notifies nobody.

use std::collections::BTreeMap;

use crate::address::{ActorAddress, TypeName};
use crate::retention::{Keep, Retention};
use crate::runtime::limits::MAX_DEPTH;
use crate::store::{
    ActorCreate, Backend, CommitRequest, CommitResult, DeleteResult, Message, MessageId, OutcomeStatus,
    OutgoingMessage, SetWatchResult, StateChange, StoreError, TurnOutcome,
};
use crate::watch::{Failed, FailureCause, StopCause, Stopped, WatchChange, WatchTarget};
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

/// The type of the watchers: never claimed by [`Env::claim`], so their notifications stay in their mailboxes.
static W: LazyLock<TypeName> = LazyLock::new(|| type_name("w"));

fn at(key: &str) -> ActorAddress {
    ActorAddress::of(&TYPE, key).expect("a valid address")
}

fn target(text: &str) -> WatchTarget {
    WatchTarget::parse(text).expect("a valid target")
}

fn event(id: &str) -> MessageId {
    MessageId::new(id).expect("a valid id")
}

fn set(text: &str) -> WatchChange {
    WatchChange { target: target(text), set: true }
}

fn unset(text: &str) -> WatchChange {
    WatchChange { target: target(text), set: false }
}

/// Creates the watcher `w/key`.
async fn watcher<F: StoreFixture>(t: &Env<F>, key: &str) -> ActorAddress {
    t.new_actor_named(&W, key, json!({})).await
}

/// Sets the watch of `watcher` on `on` from outside and asserts that it is new.
async fn watch(store: &dyn Backend, on: &str, watcher: &ActorAddress) {
    assert_eq!(store.watch(&target(on), watcher).await.unwrap(), SetWatchResult::Created, "{on} by {watcher}");
}

/// Every stored watch as `(target, watcher)` text, following every page of `limit`.
async fn watches(
    store: &dyn Backend,
    watcher: Option<&ActorAddress>,
    on: Option<&str>,
    limit: usize,
) -> Vec<(String, String)> {
    let on = on.map(target);
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = store.watches(watcher, on.as_ref(), cursor.as_ref(), limit).await.unwrap();
        out.extend(page.items.into_iter().map(|w| (w.target.to_string(), w.watcher.to_string())));
        match page.next {
            Some(next) => cursor = Some(next),
            None => return out,
        }
    }
}

fn pair(on: &str, watcher: &str) -> (String, String) {
    (on.to_owned(), watcher.to_owned())
}

/// The notifications pending at `address`.
async fn notifications<F: StoreFixture>(t: &Env<F>, address: &ActorAddress) -> Vec<Message> {
    t.pending(address).await.into_iter().filter(|m| m.notification).collect()
}

fn failed_of(m: &Message) -> Failed {
    assert_eq!(m.message_type, "Failed", "{m:?}");
    serde_json::from_value(m.payload.clone()).expect("a Failed")
}

fn stopped_of(m: &Message) -> Stopped {
    assert_eq!(m.message_type, "Stopped", "{m:?}");
    serde_json::from_value(m.payload.clone()).expect("a Stopped")
}

/// Sends `message` to its target, claims it (the only due message of [`TYPE`]) and commits it with `outcome` as the
/// event `id`.
async fn end_with<F: StoreFixture>(t: &Env<F>, message: OutgoingMessage, outcome: TurnOutcome, id: &str) -> Message {
    let sent = enqueue_ok(&*t.store(), message).await;
    let c = t.claim_one().await;
    assert_eq!(c.message.id, sent.id);
    t.commit_ok(CommitRequest { event: event(id), ..ending(&c, outcome) }).await;
    c.message
}

/// Fails a new message to `to` as the event `id`; returns the message as it was claimed.
async fn fail<F: StoreFixture>(t: &Env<F>, to: &ActorAddress, id: &str) -> Message {
    end_with(t, OutgoingMessage::new(to.clone(), "Msg", json!({"n": 1})), TurnOutcome::Failed("boom".into()), id).await
}

/// watches - a commit sets and removes watches of its actor in the order of the calls, and a failed commit sets none
pub async fn watches_a_commit_sets_and_removes_watches_of_its_actor_in_the_order_of_the_calls<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor_named(&TYPE, "a", json!({})).await;
        t.send(&a).await;
        let c = t.claim_one().await;
        let changes = vec![
            set("worker/w1"),
            set("billing"),
            unset("billing"),
            set("*"),
            unset("worker/w2"),
            unset("other/x"),
            set("other/x"),
            set("worker/w1"),
        ];
        t.commit_ok(CommitRequest { watches: changes, ..done(&c) }).await;
        let held = watches(&*store, Some(&a), None, 100).await;
        let me = a.as_str();
        assert_eq!(held, [pair("*", me), pair("other/x", me), pair("worker/w1", me)], "the last call stands");

        // A commit that is rejected, and one that lost its lease, set and remove nothing.
        t.send(&a).await;
        let c = t.claim_one().await;
        let rejected = CommitRequest {
            watches: vec![set("billing"), unset("*")],
            sends: vec![OutgoingMessage::new(at("absent"), "Msg", json!(0))],
            ..done(&c)
        };
        assert_matches!(t.commit(rejected).await, CommitResult::Rejected(_));
        let lost = CommitRequest { epoch: c.epoch + 1_000, watches: vec![set("billing"), unset("*")], ..done(&c) };
        assert_eq!(t.commit(lost).await, CommitResult::LeaseLost);
        assert_eq!(watches(&*store, Some(&a), None, 100).await, held, "nothing changed");
        t.commit_ok(done(&c)).await; // the lease is still held
    })
    .await
}

/// watches - watches are set, removed and listed from outside, with both filters and paging, and a second set changes
/// nothing
pub async fn watches_are_set_removed_and_listed_from_outside_with_both_filters_and_paging<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let (a, b) = (watcher(t, "a").await, watcher(t, "b").await);
        watch(&*store, "billing", &a).await;
        assert_eq!(store.watch(&target("billing"), &a).await.unwrap(), SetWatchResult::Exists, "idempotent");
        watch(&*store, "billing.invoices/acme", &a).await;
        watch(&*store, "billing", &b).await;
        watch(&*store, "*", &b).await;
        watch(&*store, "conformance/nosuch", &b).await; // a target need not exist
        let absent = ActorAddress::of(&W, "absent").unwrap();
        assert_eq!(store.watch(&target("billing"), &absent).await.unwrap(), SetWatchResult::NoSuchWatcher);

        let all = vec![
            pair("*", "w/b"),
            pair("billing", "w/a"),
            pair("billing", "w/b"),
            pair("billing.invoices/acme", "w/a"),
            pair("conformance/nosuch", "w/b"),
        ];
        assert_eq!(watches(&*store, None, None, 100).await, all, "in the order of target and watcher");
        assert_eq!(watches(&*store, None, None, 1).await, all, "across pages of one");
        assert_eq!(watches(&*store, None, None, 2).await, all, "across pages of two");
        let first = store.watches(None, None, None, 2).await.unwrap();
        assert_eq!(first.items.len(), 2);
        assert!(first.next.is_some());
        let by_a = vec![pair("billing", "w/a"), pair("billing.invoices/acme", "w/a")];
        assert_eq!(watches(&*store, Some(&a), None, 1).await, by_a, "of one watcher");
        let on_billing = vec![pair("billing", "w/a"), pair("billing", "w/b")];
        assert_eq!(watches(&*store, None, Some("billing"), 1).await, on_billing, "of exactly one target");
        assert_eq!(watches(&*store, Some(&b), Some("billing"), 100).await, [pair("billing", "w/b")], "both");
        assert!(watches(&*store, Some(&a), Some("*"), 100).await.is_empty(), "a pair that is not stored");
        assert!(matches!(store.watches(None, None, None, 0).await, Err(StoreError::Invalid(_))), "a limit of 0");
        let bad = crate::store::PageCursor("no-space".into());
        assert!(matches!(store.watches(None, None, Some(&bad), 10).await, Err(StoreError::Invalid(_))), "a bad cursor");

        assert!(store.unwatch(&target("billing"), &a).await.unwrap());
        assert!(!store.unwatch(&target("billing"), &a).await.unwrap(), "not stored any more");
        assert!(!store.unwatch(&target("billing"), &absent).await.unwrap());
        assert_eq!(watches(&*store, Some(&a), None, 100).await, [pair("billing.invoices/acme", "w/a")]);
    })
    .await
}

/// watches - a failed commit writes one `Failed` per watcher, with every field, the sender, no `causedBy` and the mark
pub async fn watches_a_failed_commit_writes_one_failed_per_watcher_with_every_field_and_the_mark<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let sender = t.new_actor_named(&OTHER, "s", json!({})).await;
        let (b, a) = (watcher(t, "b").await, watcher(t, "a").await);
        watch(&*store, x.as_str(), &b).await;
        watch(&*store, x.as_str(), &a).await;
        let message = OutgoingMessage {
            from: Some(sender.clone()),
            correlation_id: Some("run-7".into()),
            headers: BTreeMap::from([("h".to_owned(), "v".to_owned())]),
            ..OutgoingMessage::new(x.clone(), "Charge", json!({"amount": 3}))
        };
        let failed = end_with(t, message, TurnOutcome::Failed("declined".into()), "e1").await;

        for (w, n) in [(&a, 0), (&b, 1)] {
            let got = notifications(t, w).await;
            assert_eq!(got.len(), 1, "{w}");
            let m = &got[0];
            assert_eq!(m.id.as_str(), format!("e1.{n}"), "named after the event, in the order of the watchers");
            assert_eq!(m.to, *w);
            assert_eq!(m.from.as_ref(), Some(&x), "sent by the actor of the event");
            assert_eq!(m.caused_by, None, "a Failed has no causedBy");
            assert!(m.notification, "marked");
            assert!(m.headers.is_empty() && m.retry.is_none() && m.correlation_id.is_none());
            assert_eq!(m.retention, None, "the failed message has no retention");
            assert_eq!((m.attempt, m.failures), (0, 0));
            assert_eq!(
                failed_of(m),
                Failed {
                    actor: x.clone(),
                    message_id: failed.id.clone(),
                    message_type: "Charge".into(),
                    payload: json!({"amount": 3}),
                    correlation_id: Some("run-7".into()),
                    from: Some(sender.clone()),
                    error: "declined".into(),
                    attempts: 1,
                    cause: FailureCause::Handler,
                }
            );
            assert_eq!(m.payload["correlationId"], "run-7");
        }
        let outcome = outcome_record(&*store, &failed.id).await.expect("an outcome");
        assert!(!outcome.envelope.notification, "the failed message is not a notification");

        // Without a correlation id and from outside a turn, both are written as null.
        let bare = fail(t, &x, "e2").await;
        let got = notifications(t, &a).await;
        let m = got.iter().find(|m| m.id.as_str() == "e2.0").expect("the second Failed");
        assert_eq!((m.payload["correlationId"].clone(), m.payload["from"].clone()), (json!(null), json!(null)));
        assert_eq!(failed_of(m).message_id, bare.id);
    })
    .await
}

/// watches - a failed commit writes the cause it was given into the `Failed` of every watcher, and its error stays as
/// it was given
pub async fn watches_a_failed_commit_writes_the_cause_it_was_given<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let (a, b) = (watcher(t, "a").await, watcher(t, "b").await);
        watch(&*store, x.as_str(), &a).await;
        watch(&*store, "*", &b).await;
        let causes = [
            FailureCause::Handler,
            FailureCause::Node,
            FailureCause::Refused,
            FailureCause::Timeout,
            FailureCause::Message,
            FailureCause::State,
            FailureCause::Commit,
            FailureCause::ClaimLimit,
        ];
        for (n, cause) in causes.into_iter().enumerate() {
            enqueue_ok(&*store, OutgoingMessage::new(x.clone(), "Msg", json!({"n": n}))).await;
            let c = t.claim_one().await;
            let failed = TurnOutcome::Failed(format!("failed for {cause}"));
            t.commit_ok(CommitRequest { event: event(&format!("e{n}")), ..ending(&c, failed).with_cause(cause) }).await;
            for w in [&a, &b] {
                let got = notifications(t, w).await;
                let m = got.iter().find(|m| m.id.as_str().starts_with(&format!("e{n}."))).expect("a Failed");
                let told = failed_of(m);
                assert_eq!((told.cause, told.error.as_str()), (cause, &*format!("failed for {cause}")), "{w}");
                assert_eq!(m.payload["cause"], json!(cause.name()), "in kebab-case");
            }
        }
    })
    .await
}

/// watches - a `Failed` is kept by the `failed` rule of the failed message's retention, for both of its outcomes
pub async fn watches_a_failed_is_kept_by_the_failed_rule_of_the_retention_of_the_failed_message<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let w = watcher(t, "a").await;
        watch(&*store, x.as_str(), &w).await;
        let kept = |retention: Option<Retention>| OutgoingMessage {
            retention,
            ..OutgoingMessage::new(x.clone(), "Msg", json!({"secret": 1}))
        };
        let boom = || TurnOutcome::Failed("boom".into());
        end_with(t, kept(Some(Retention::REMOVE_ON_FAILED)), boom(), "e1").await;
        end_with(t, kept(Some(Retention::REMOVE_ON_DONE)), boom(), "e2").await;
        let age = Keep::For { age: std::time::Duration::from_secs(3600) };
        end_with(t, kept(Some(Retention { done: None, failed: Some(age.clone()) })), boom(), "e3").await;
        let got = notifications(t, &w).await;
        let retention_of = |id: &str| got.iter().find(|m| m.id.as_str() == id).expect(id).retention.clone();
        assert_eq!(retention_of("e1.0"), Some(Retention { done: Some(Keep::Remove), failed: Some(Keep::Remove) }));
        assert_eq!(retention_of("e2.0"), None, "the failed message keeps a failure by the store default");
        assert_eq!(retention_of("e3.0"), Some(Retention { done: Some(age.clone()), failed: Some(age) }));

        // The watcher handles the first: although it ends `done`, its outcome is removed with the failure's rule.
        let c = t.claim_one_types(&[&W]).await;
        assert_eq!(c.message.id.as_str(), "e1.0");
        t.commit_ok(done(&c)).await;
        assert_eq!(outcome_record(&*store, &c.message.id).await, None, "hidden at once");
        let c = t.claim_one_types(&[&W]).await;
        assert_eq!(c.message.id.as_str(), "e2.0");
        t.commit_ok(done(&c)).await;
        assert!(outcome_record(&*store, &c.message.id).await.is_some(), "kept by the store default");
    })
    .await
}

/// watches - a rescheduled, a done and a cancelled commit write nothing
pub async fn watches_a_rescheduled_a_done_and_a_cancelled_commit_write_nothing<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let w = watcher(t, "a").await;
        watch(&*store, x.as_str(), &w).await;
        watch(&*store, "*", &w).await;
        let message = || OutgoingMessage::new(x.clone(), "Msg", json!({}));
        let again = TurnOutcome::Rescheduled { delivery: crate::store::Delivery::Now, error: "again".into() };
        end_with(t, message(), again, "e1").await;
        let c = t.claim_one().await; // the rescheduled message
        t.commit_ok(CommitRequest { event: event("e2"), ..done(&c) }).await;
        end_with(t, message(), TurnOutcome::Cancelled, "e3").await;
        assert!(t.pending(&w).await.is_empty(), "nobody is told");
    })
    .await
}

/// watches - a type prefix matches at a dot, `*` matches every type, and neither is told of a removal
pub async fn watches_a_type_prefix_matches_at_a_dot_and_every_type_matches_and_neither_is_told_of_a_removal<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let (prefix, every) = (watcher(t, "prefix").await, watcher(t, "every").await);
        watch(&*store, "billing", &prefix).await;
        watch(&*store, "*", &every).await;
        let types = ["billing", "billing.invoices", "billings"].map(type_name);
        let mut failed = Vec::new();
        for (i, ty) in types.iter().enumerate() {
            let x = t.new_actor_named(ty, "x", json!({})).await;
            enqueue_ok(&*store, OutgoingMessage::new(x.clone(), "Msg", json!({}))).await;
            let c = t.claim_one_types(&[ty]).await;
            let id = event(&format!("e{i}"));
            t.commit_ok(CommitRequest { event: id, ..ending(&c, TurnOutcome::Failed("boom".into())) }).await;
            failed.push(x);
        }
        let told = |ms: Vec<Message>| ms.iter().map(|m| failed_of(m).actor.to_string()).collect::<Vec<_>>();
        assert_eq!(told(notifications(t, &prefix).await), ["billing/x", "billing.invoices/x"], "not billings");
        assert_eq!(told(notifications(t, &every).await), ["billing/x", "billing.invoices/x", "billings/x"]);

        for x in &failed {
            assert_matches!(store.delete(x, &event(&format!("d-{x}"))).await.unwrap(), DeleteResult::Deleted { .. });
        }
        assert_eq!(notifications(t, &prefix).await.len(), 2, "no Stopped for a type target");
        assert_eq!(notifications(t, &every).await.len(), 3, "no Stopped for every type");
    })
    .await
}

/// watches - a stop and a delete write one `Stopped` per watcher of each removed actor, with the cause, `by` and, for a
/// stop, the `causedBy`
pub async fn watches_a_stop_and_a_delete_write_one_stopped_per_watcher_of_each_removed_actor<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let w = watcher(t, "a").await;
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        t.send(&p).await;
        let c = t.claim_one().await;
        let kid = at("c");
        let child = ActorCreate { address: kid.clone(), state: json!({}), child: true, pinned_to: None };
        t.commit_ok(CommitRequest { creates: vec![child], ..done(&c) }).await;
        watch(&*store, p.as_str(), &w).await;
        watch(&*store, kid.as_str(), &w).await;
        watch(&*store, "conformance", &w).await; // a type target: no Stopped

        t.send(&p).await;
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest { event: event("e1"), state: StateChange::Stop, ..done(&c) }).await;
        let got = notifications(t, &w).await;
        let ids: Vec<&str> = got.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["e1.0", "e1.1"], "in the order of the removed actors");
        assert_eq!(stopped_of(&got[0]), Stopped { actor: kid.clone(), cause: StopCause::Parent, by: Some(p.clone()) });
        assert_eq!(stopped_of(&got[1]), Stopped { actor: p.clone(), cause: StopCause::Stopped, by: Some(p.clone()) });
        for m in &got {
            assert_eq!(m.caused_by.as_ref(), Some(&c.message.id), "caused by the turn's message");
            assert!(m.notification && m.retention.is_none());
        }
        assert_eq!((got[0].from.as_ref(), got[1].from.as_ref()), (Some(&kid), Some(&p)), "sent by the removed actor");

        // A delete from outside: no `by` and no `causedBy`.
        let q = t.new_actor_named(&TYPE, "q", json!({})).await;
        t.send(&q).await;
        let c = t.claim_one().await;
        let d = at("d");
        let child = ActorCreate { address: d.clone(), state: json!({}), child: true, pinned_to: None };
        t.commit_ok(CommitRequest { creates: vec![child], ..done(&c) }).await;
        watch(&*store, q.as_str(), &w).await;
        watch(&*store, d.as_str(), &w).await;
        assert_eq!(store.delete(&q, &event("e2")).await.unwrap(), DeleteResult::Deleted { purged: 0 });
        let got: Vec<Message> = notifications(t, &w).await.into_iter().filter(|m| m.id.as_str() > "e2").collect();
        let ids: Vec<&str> = got.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["e2.0", "e2.1"]);
        assert_eq!(stopped_of(&got[0]), Stopped { actor: d.clone(), cause: StopCause::Parent, by: None });
        assert_eq!(stopped_of(&got[1]), Stopped { actor: q.clone(), cause: StopCause::Deleted, by: None });
        assert!(got.iter().all(|m| m.caused_by.is_none()));
        assert_eq!(got[1].payload["by"], json!(null), "always written");
    })
    .await
}

/// watches - a watcher that is removed in the same transaction gets nothing, and its watches are gone
pub async fn watches_a_watcher_removed_in_the_same_transaction_gets_nothing_and_its_watches_are_gone<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let other = t.new_actor_named(&OTHER, "o", json!({})).await;
        for (round, by_stop) in [(0, false), (1, true)] {
            let p = t.new_actor_named(&TYPE, &format!("p{round}"), json!({})).await;
            t.send(&p).await;
            let c = t.claim_one().await;
            let kid = at(&format!("p{round}-kid"));
            let child = ActorCreate { address: kid.clone(), state: json!({}), child: true, pinned_to: None };
            let changes = vec![set(kid.as_str()), set(other.as_str()), set(p.as_str())];
            t.commit_ok(CommitRequest { creates: vec![child], watches: changes, ..done(&c) }).await;
            assert_eq!(watches(&*store, Some(&p), None, 100).await.len(), 3);
            if by_stop {
                t.send(&p).await;
                let c = t.claim_one().await;
                t.commit_ok(CommitRequest { state: StateChange::Stop, ..done(&c) }).await;
            } else {
                assert_matches!(store.delete(&p, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. });
            }
            assert!(watches(&*store, Some(&p), None, 100).await.is_empty(), "round {round}: its watches go");
            assert!(store.list("", None, 100).await.unwrap().items.iter().all(|a| *a != p && *a != kid));
        }
        assert!(t.pending(&other).await.is_empty(), "nothing was written to anyone");
    })
    .await
}

/// watches - removing an actor removes the watches it holds, and the watches on it stay over a delete and a new create
pub async fn watches_removing_an_actor_removes_the_watches_it_holds_and_the_watches_on_it_stay<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let w = watcher(t, "a").await;
        for on in ["conformance/t1", "billing", "*"] {
            watch(&*store, on, &w).await;
        }
        assert_matches!(store.delete(&w, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. });
        assert!(watches(&*store, None, None, 100).await.is_empty(), "the watches of a deleted watcher go");

        let w = watcher(t, "b").await;
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        watch(&*store, x.as_str(), &w).await;
        assert_matches!(store.delete(&x, &event("e1")).await.unwrap(), DeleteResult::Deleted { .. });
        assert_eq!(watches(&*store, None, None, 100).await, [pair(x.as_str(), w.as_str())], "the watch stays");
        t.new_actor_named(&TYPE, "x", json!({})).await;
        fail(t, &x, "e2").await;
        let got = notifications(t, &w).await;
        let kinds: Vec<(&str, &str)> = got.iter().map(|m| (m.id.as_str(), m.message_type.as_str())).collect();
        assert_eq!(kinds, [("e1.0", "Stopped"), ("e2.0", "Failed")], "told of the Stopped and of the Failed");
    })
    .await
}

/// watches - a notification that fails writes nothing, and a message of the type `Failed` that a sender wrote is not
/// marked
pub async fn watches_a_notification_that_fails_writes_nothing_and_a_sender_cannot_mark_a_message<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let w = watcher(t, "a").await;
        watch(&*store, x.as_str(), &w).await;
        watch(&*store, w.as_str(), &x).await; // they watch each other
        fail(t, &x, "e1").await;
        let c = t.claim_one_types(&[&W]).await;
        assert!(c.message.notification);
        t.commit_ok(CommitRequest { event: event("e2"), ..ending(&c, TurnOutcome::Failed("cannot".into())) }).await;
        assert!(t.pending(&x).await.is_empty(), "the failed notification notifies nobody");
        assert!(t.pending(&w).await.is_empty(), "and nothing more is written");

        // A sender that writes a message of the type Failed does not mark it; its failure notifies the watchers.
        let fake = OutgoingMessage::new(x.clone(), "Failed", json!({"actor": "w/a"}));
        let sent = enqueue_ok(&*store, fake).await;
        assert!(!sent.notification, "a sender cannot mark a message");
        let c = t.claim_one().await;
        assert!(!c.message.notification);
        t.commit_ok(CommitRequest { event: event("e3"), ..ending(&c, TurnOutcome::Failed("no".into())) }).await;
        let got = notifications(t, &w).await;
        assert_eq!(got.len(), 1);
        assert_eq!(failed_of(&got[0]).message_id, sent.id);
    })
    .await
}

/// watches - the notifications of one event have the ids of the event in the order of the actor's and then the
/// watcher's address, one per watcher and actor
pub async fn watches_the_notifications_of_one_event_are_numbered_in_order_one_per_watcher_and_actor<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let p = t.new_actor_named(&TYPE, "p", json!({})).await;
        t.send(&p).await;
        let c = t.claim_one().await;
        let kid = at("c");
        let child = ActorCreate { address: kid.clone(), state: json!({}), child: true, pinned_to: None };
        t.commit_ok(CommitRequest { creates: vec![child], ..done(&c) }).await;
        let (b, a) = (watcher(t, "b").await, watcher(t, "a").await);
        watch(&*store, kid.as_str(), &a).await;
        watch(&*store, kid.as_str(), &b).await;
        watch(&*store, p.as_str(), &b).await;
        assert_matches!(store.delete(&p, &event("e1")).await.unwrap(), DeleteResult::Deleted { .. });
        let mut all: Vec<(String, String, String)> = Vec::new();
        for w in [&a, &b] {
            for m in notifications(t, w).await {
                all.push((m.id.to_string(), stopped_of(&m).actor.to_string(), w.to_string()));
            }
        }
        all.sort();
        let expected =
            [("e1.0", "conformance/c", "w/a"), ("e1.1", "conformance/c", "w/b"), ("e1.2", "conformance/p", "w/b")]
                .map(|(id, about, w)| (id.to_owned(), about.to_owned(), w.to_owned()));
        assert_eq!(all, expected);

        // Two watches of one watcher that match one failure give one notification.
        let x = ActorAddress::of(&type_name("billing.invoices"), "acme").unwrap();
        t.new_actor_named(x.type_name(), "acme", json!({})).await;
        let both = watcher(t, "both").await;
        watch(&*store, "billing", &both).await;
        watch(&*store, x.as_str(), &both).await;
        enqueue_ok(&*store, OutgoingMessage::new(x.clone(), "Msg", json!({}))).await;
        let c = t.claim_one_types(&[x.type_name()]).await;
        t.commit_ok(CommitRequest { event: event("e2"), ..ending(&c, TurnOutcome::Failed("boom".into())) }).await;
        let got = notifications(t, &both).await;
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["e2.0"], "one, not two");
    })
    .await
}

/// watches - an actor that watches itself is told of its failures and gets nothing when it stops
pub async fn watches_an_actor_that_watches_itself_is_told_of_its_failures_and_gets_nothing_when_it_stops<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let other = watcher(t, "o").await;
        watch(&*store, x.as_str(), &x).await;
        watch(&*store, x.as_str(), &other).await;
        let failed = fail(t, &x, "e1").await;
        let got = notifications(t, &x).await;
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].id.as_str(), failed_of(&got[0]).message_id), ("e1.0", failed.id), "x sorts before w/o");
        let c = t.claim_one().await; // its own Failed
        t.commit_ok(CommitRequest { event: event("e2"), state: StateChange::Stop, ..done(&c) }).await;
        let got = notifications(t, &other).await;
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["e1.1", "e2.0"], "only the other is told");
        assert!(store.get(&x).await.unwrap().is_none());
        assert!(watches(&*store, Some(&x), None, 100).await.is_empty());
    })
    .await
}

/// watches - a notification whose id is already in use is skipped: the commit succeeds, and the other notifications of
/// the event are written
pub async fn watches_a_notification_whose_id_is_in_use_is_skipped_and_the_others_are_written<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let (a, b) = (watcher(t, "a").await, watcher(t, "b").await);
        watch(&*store, x.as_str(), &a).await;
        watch(&*store, x.as_str(), &b).await;
        let other = t.new_actor_named(&OTHER, "o", json!({})).await;
        let taken = t.send_id(&other, event("e1.0")).await; // the id the notification for w/a would get
        let failed = fail(t, &x, "e1").await; // asserts that the commit succeeds
        assert!(notifications(t, &a).await.is_empty(), "skipped");
        let got = notifications(t, &b).await;
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["e1.1"], "the others are written");
        assert_eq!(failed_of(&got[0]).message_id, failed.id);
        assert_eq!(t.pending(&other).await, [taken], "the message that holds the id is left as it was");
    })
    .await
}

/// watches - a watcher that does not exist is skipped and loses all its watches, not only the one that matched
pub async fn watches_a_watcher_that_does_not_exist_is_skipped_and_loses_all_its_watches<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        if !assume(t.fixture.supports_removing_rows(), "needs to remove a row by hand") {
            return;
        }
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let (gone, kept) = (watcher(t, "gone").await, watcher(t, "kept").await);
        for on in [x.as_str(), "billing", "*"] {
            watch(&*store, on, &gone).await;
        }
        watch(&*store, x.as_str(), &kept).await;
        t.fixture.remove_row(&gone).await;
        assert_eq!(store.get(&gone).await.unwrap(), None, "the row is gone");
        assert_eq!(watches(&*store, Some(&gone), None, 100).await.len(), 3, "its watches stay until an event");

        let failed = fail(t, &x, "e1").await; // asserts that the commit succeeds
        assert_eq!(outcome_record(&*store, &failed.id).await.expect("an outcome").status, OutcomeStatus::Failed);
        assert!(watches(&*store, Some(&gone), None, 100).await.is_empty(), "all three watches are removed");
        assert_eq!(watches(&*store, None, None, 100).await, [pair(x.as_str(), kept.as_str())], "the others stay");
        let got = notifications(t, &kept).await;
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["e1.0"], "numbered without the gone one");
    })
    .await
}

/// watches - a delete racing a send from a watcher whose address sorts before its target's ends without an error, and
/// the watcher is told
pub async fn watches_a_delete_racing_a_send_from_a_watcher_that_sorts_first_ends_without_an_error<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..50 {
            let w = t.new_actor_named(&TYPE, &format!("a{round}"), json!({})).await;
            let x = t.new_actor_named(&TYPE, &format!("t{round}"), json!({})).await;
            assert!(w < x, "the watcher sorts first");
            watch(&*store, x.as_str(), &w).await;
            t.send(&w).await;
            let c = t.claim_one().await;
            let results = t
                .concurrently(2, |i, peer| {
                    let (c, x) = (c.clone(), x.clone());
                    async move {
                        if i == 0 {
                            let send = OutgoingMessage::new(x.clone(), "Msg", json!(0));
                            Err(peer.commit(CommitRequest { sends: vec![send], ..done(&c) }).await.unwrap())
                        } else {
                            Ok(peer.delete(&x, &MessageId::random()).await.unwrap())
                        }
                    }
                })
                .await;
            for result in results {
                match result {
                    Err(commit) => assert!(
                        matches!(commit, CommitResult::Committed { .. } | CommitResult::Rejected(_)),
                        "round {round}: {commit:?}"
                    ),
                    Ok(deleted) => assert!(matches!(deleted, DeleteResult::Deleted { .. }), "round {round}"),
                }
            }
            let got = notifications(t, &w).await;
            assert_eq!(got.len(), 1, "round {round}: {got:?}");
            assert_eq!(stopped_of(&got[0]).actor, x, "round {round}");
            assert_matches!(store.delete(&w, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. });
        }
    })
    .await
}

/// watches - a watch set while a message of its target fails ends without an error, and a watch stored before the
/// event tells its watcher
pub async fn watches_a_watch_set_while_a_message_of_its_target_fails_ends_without_an_error<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..50 {
            let x = t.new_actor_named(&TYPE, &format!("t{round}"), json!({})).await;
            let early = watcher(t, &format!("early{round}")).await;
            let late = watcher(t, &format!("late{round}")).await;
            watch(&*store, x.as_str(), &early).await;
            t.send(&x).await;
            let c = t.claim_one().await;
            let failure =
                CommitRequest { event: event(&format!("e{round}")), ..ending(&c, TurnOutcome::Failed("boom".into())) };
            let results = t
                .concurrently(2, |i, peer| {
                    let (failure, x, late) = (failure.clone(), x.clone(), late.clone());
                    async move {
                        if i == 0 {
                            Err(peer.commit(failure).await.unwrap())
                        } else {
                            Ok(peer.watch(&WatchTarget::Actor(x), &late).await.unwrap())
                        }
                    }
                })
                .await;
            for result in results {
                match result {
                    Err(commit) => assert!(matches!(commit, CommitResult::Committed { .. }), "round {round}"),
                    Ok(set) => assert_eq!(set, SetWatchResult::Created, "round {round}"),
                }
            }
            assert_eq!(notifications(t, &early).await.len(), 1, "round {round}: stored before the event");
            assert!(notifications(t, &late).await.len() <= 1, "round {round}");
            for a in [&early, &late, &x] {
                assert_matches!(store.delete(a, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. });
            }
        }
    })
    .await
}

/// A value `levels` deep: arrays and objects in turn around a number, built without recursion.
fn nested(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, level| {
        if level % 2 == 0 { Value::Array(vec![inner]) } else { Value::Object(Map::from_iter([("n".into(), inner)])) }
    })
}

/// How deep a value of [`nested`] is, counted without recursion.
fn depth_of(mut value: &Value) -> usize {
    let mut levels = 0;
    loop {
        let inner = match value {
            Value::Array(items) => items.first(),
            Value::Object(fields) => fields.values().next(),
            _ => None,
        };
        match inner {
            Some(inner) => (levels, value) = (levels + 1, inner),
            None => return levels,
        }
    }
}

/// watches - a message whose payload is at the depth limit fails, and its `Failed`, a level deeper, is written and
/// claimed whole
pub async fn watches_the_failed_of_a_payload_at_the_depth_limit_is_written_and_claimed_whole<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let x = t.new_actor_named(&TYPE, "x", json!({})).await;
        let w = watcher(t, "a").await;
        watch(&*store, x.as_str(), &w).await;
        let payload = nested(MAX_DEPTH);
        assert_eq!(depth_of(&payload), MAX_DEPTH);
        let message = OutgoingMessage::new(x.clone(), "Deep", payload.clone());
        let failed = end_with(t, message, TurnOutcome::Failed("declined".into()), "e1").await;
        assert_eq!(failed.payload, payload, "the message is read whole");

        let c = t.claim_one_types(&[&W]).await;
        assert_eq!((c.message.id.as_str(), c.message.to.as_str()), ("e1.0", w.as_str()));
        assert_eq!(depth_of(&c.message.payload["payload"]), MAX_DEPTH, "the notice is a level deeper: 65");
        let told = failed_of(&c.message);
        assert_eq!((told.message_id, told.payload), (failed.id, payload));
        t.commit_ok(done(&c)).await;
    })
    .await
}

/// The numbers of the numeric case: a float that a parser without exact rounding reads one bit off, and its negative,
/// the smallest subnormal, the largest finite f64, a negative zero and the bounds of i64 and u64.
fn numbers() -> Value {
    let floats = [1.0715660391465826e-75, -1.0715660391465826e-75, f64::from_bits(1), f64::MAX, -0.0];
    let mut all: Vec<Value> = floats.into_iter().map(Value::from).collect();
    all.extend([Value::from(i64::MIN), Value::from(i64::MAX), Value::from(u64::MAX)]);
    Value::Array(all)
}

/// Asserts that `got` holds [`numbers`] exactly: integers as the same integers and floats with the same bits, the
/// negative zero as a zero where the backend normalizes it.
fn assert_numbers(got: &Value, normalizes_negative_zero: bool, what: &str) {
    let (got, sent) = (got.as_array().expect(what), numbers());
    let sent = sent.as_array().unwrap();
    assert_eq!(got.len(), sent.len(), "{what}");
    for (got, sent) in got.iter().zip(sent) {
        if sent.is_f64() {
            let mut bits = sent.as_f64().unwrap().to_bits();
            if normalizes_negative_zero && bits == (-0.0f64).to_bits() {
                bits = 0.0f64.to_bits();
            }
            let read = got.as_f64().unwrap_or_else(|| panic!("{what}: {got} is not a number"));
            assert_eq!((read.to_bits(), got.is_f64()), (bits, true), "{what}: {sent} was read as {got}");
        } else {
            assert_eq!((got, got.is_f64()), (sent, false), "{what}: an exact integer");
        }
    }
}

/// watches - numbers in a state, in a payload and in the `Failed` of that payload are read back exactly: integers as
/// they were and floats with the same bits; PostgreSQL stores a negative zero as a zero
pub async fn watches_numbers_in_a_state_a_payload_and_its_failed_are_read_back_exactly<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let normalizes = t.fixture.normalizes_negative_zero();
        let x = t.new_actor_named(&TYPE, "x", json!({"n": numbers()})).await;
        let record = store.get(&x).await.unwrap().expect("the actor");
        assert_numbers(&record.state["n"], normalizes, "the state");
        let w = watcher(t, "a").await;
        watch(&*store, x.as_str(), &w).await;

        let message = OutgoingMessage::new(x.clone(), "Numbers", json!({"n": numbers()}));
        enqueue_ok(&*store, message).await;
        assert_numbers(&t.pending(&x).await[0].payload["n"], normalizes, "the pending payload");
        let c = t.claim_one().await;
        assert_numbers(&c.message.payload["n"], normalizes, "the claimed payload");
        t.commit_ok(CommitRequest { state: StateChange::NewState(json!({"m": numbers()})), ..done(&c) }).await;
        let record = store.get(&x).await.unwrap().expect("the actor");
        assert_numbers(&record.state["m"], normalizes, "the state a commit wrote");

        let message = OutgoingMessage::new(x.clone(), "Numbers", json!({"n": numbers()}));
        end_with(t, message, TurnOutcome::Failed("boom".into()), "e1").await;
        let c = t.claim_one_types(&[&W]).await;
        assert_numbers(&failed_of(&c.message).payload["n"], normalizes, "the payload of the Failed");
        t.commit_ok(done(&c)).await;
    })
    .await
}
