//! A column added under an open store (ADR 0025): a store runs the operations of a node, another connection adds a
//! column with a default to every table of the schema, and the store runs them again. A statement that reads with `*`
//! and that a connection has prepared fails then on PostgreSQL; each SQL store runs this in its own tests.

use std::collections::BTreeSet;
use std::time::Duration;

use crate::address::{ActorAddress, TypeName};
use crate::store::{
    Backend, CancelResult, CommitRequest, CreateResult, DeleteResult, DeliverNowResult, Delivery, MessageStatus,
    NodeId, OutcomeStatus, OutgoingMessage, StateChange, TurnOutcome,
};
use crate::tests::conformance::{committed, enqueue_ok, type_def};
use crate::watch::WatchTarget;
use serde_json::json;

/// The column every table gets: one with a default, as a migration that may be additive adds it.
pub const ADDED_COLUMN: &str = "added_later integer NOT NULL DEFAULT 7";

/// Runs the operations a node runs on `store` once, checking each answer; `round` keeps two rounds apart.
pub async fn operations(store: &dyn Backend, round: &str) {
    let name = TypeName::parse("added").unwrap();
    let types = BTreeSet::from([name.clone()]);
    let node = NodeId::new(format!("node-{round}")).unwrap();
    let at = |key: &str| ActorAddress::of(&name, &format!("{round}-{key}")).unwrap();
    let (actor, watcher) = (at("actor"), at("watcher"));
    let ttl = Duration::from_secs(3600);

    store.register(type_def(&name, "added")).await.unwrap();
    assert!(store.list_types().await.unwrap().iter().any(|t| t.name == name));
    assert_eq!(store.create(&actor, json!({"n": 0})).await.unwrap(), CreateResult::Created);
    assert_eq!(store.create(&watcher, json!({})).await.unwrap(), CreateResult::Created);
    assert_eq!(store.get(&actor).await.unwrap().expect("created").state, json!({"n": 0}));
    assert!(store.list(&format!("added/{round}-"), None, 10).await.unwrap().items.contains(&actor));
    assert!(store.children(&actor, None, 10).await.unwrap().items.is_empty());
    store.watch(&WatchTarget::from(actor.clone()), &watcher).await.unwrap();
    assert_eq!(store.watches(Some(&watcher), None, None, 10).await.unwrap().items.len(), 1);

    let first = enqueue_ok(store, OutgoingMessage::new(actor.clone(), "Msg", json!(1))).await;
    let second = enqueue_ok(store, OutgoingMessage::new(actor.clone(), "Msg", json!(2))).await;
    let later = |n: i64| OutgoingMessage {
        delivery: Delivery::After(Duration::from_secs(3600)),
        ..OutgoingMessage::new(actor.clone(), "Msg", json!(n))
    };
    let (to_cancel, to_deliver) = (enqueue_ok(store, later(3)).await, enqueue_ok(store, later(4)).await);
    assert_eq!(store.pending(&actor, None, 10).await.unwrap().items.len(), 4);
    assert!(store.list_pending(&format!("added/{round}-"), None, 10).await.unwrap().items.len() >= 4);
    assert!(matches!(store.status(&first.id).await.unwrap(), Some(MessageStatus::Pending(_))));
    assert!(store.next_due_at(&types).await.unwrap().is_some());

    let claims = store.claim_due(&types, &node, ttl, 10).await.unwrap();
    let claim = claims.iter().find(|c| c.actor == actor).expect("the actor is claimed");
    assert_eq!(claim.message.id, first.id);
    store.renew(&actor, claim.epoch, ttl).await.unwrap();
    assert!(store.running_turns(Some(&node), "", None, 10).await.unwrap().items.iter().any(|t| t.actor == actor));
    assert!(store.running_turn(&actor).await.unwrap().is_some());
    assert!(matches!(store.status(&first.id).await.unwrap(), Some(MessageStatus::Running { .. })));
    let mut commit = CommitRequest::new(actor.clone(), claim.epoch, first.id.clone(), TurnOutcome::Done);
    commit.state = StateChange::NewState(json!({"n": 1}));
    commit.sends = vec![OutgoingMessage {
        caused_by: Some(first.id.clone()),
        ..OutgoingMessage::new(watcher.clone(), "Msg", json!(5))
    }];
    let sent = committed(store.commit(commit).await.unwrap()).sent;
    assert_eq!(sent.len(), 1);
    assert!(matches!(store.status(&first.id).await.unwrap(), Some(MessageStatus::Finished(_))));
    assert_eq!(store.caused_by(&first.id, None, 10).await.unwrap().items.len(), 1);

    let claim = store.claim_due(&types, &node, ttl, 10).await.unwrap();
    let claim = claim.iter().find(|c| c.actor == actor).expect("the actor is claimed again");
    assert_eq!(claim.message.id, second.id);
    let failed = CommitRequest::new(actor.clone(), claim.epoch, second.id.clone(), TurnOutcome::Failed("x".into()));
    committed(store.commit(failed).await.unwrap());
    let failures = store.outcomes(Some(OutcomeStatus::Failed), "", chrono::DateTime::UNIX_EPOCH, None, 100);
    assert!(failures.await.unwrap().items.iter().any(|o| o.message_id == second.id));
    assert!(store.pending(&watcher, None, 10).await.unwrap().items.iter().any(|m| m.notification), "a Failed");

    assert!(matches!(store.cancel(&to_cancel.id).await.unwrap(), CancelResult::Cancelled(_)));
    assert!(matches!(store.deliver_now(&to_deliver.id).await.unwrap(), DeliverNowResult::Delivered(_)));
    assert!(store.set_paused(&actor, true).await.unwrap());
    assert!(store.set_paused(&actor, false).await.unwrap());
    assert_eq!(store.purge(&actor).await.unwrap(), Some(1));
    assert!(store.stats(&format!("added/{round}-")).await.unwrap().actors >= 2);
    store.heartbeat_node(&node, &types).await.unwrap();
    assert!(store.list_nodes().await.unwrap().iter().any(|n| n.id == node));
    store.remove_node(&node).await.unwrap();
    assert!(store.request_cancel(&actor).await.is_ok());
    assert!(matches!(
        store.delete(&actor, &crate::store::MessageId::random()).await.unwrap(),
        DeleteResult::Deleted { .. }
    ));
    assert!(store.unwatch(&WatchTarget::from(actor.clone()), &watcher).await.is_ok());
    store.sweep_expired().await.unwrap();
}
