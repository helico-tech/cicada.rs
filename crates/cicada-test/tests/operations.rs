//! The in-process operations behind the HTTP endpoints (Kotlin `OperationsTest`).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use cicada::address::ActorAddress;
use cicada::contract::Contract;
use cicada::runtime::{CreateActorResult, SendResult};
use cicada::schema;
use cicada::store::{CancelResult, DeleteResult, Delivery, MessageId, MessageStatus, OutcomeStatus, Store};
use cicada::time;
use cicada::turn::SendOptions;
use cicada::ActorSystem;
use cicada_test::{InMemoryStore, TestActors, VirtualClock};
use common::*;
use serde_json::{Value, json};

fn a() -> Counter {
    Counter::new("a")
}

async fn harness() -> TestActors {
    TestActors::builder().with(|n| n.host(CounterActor).host(EchoActor)).build().await.unwrap()
}

fn parse(uri: &str) -> ActorAddress {
    ActorAddress::parse(uri).unwrap()
}

#[tokio::test(start_paused = true)]
async fn create_created_exists_no_such_type_invalid_state() {
    let test = harness().await;
    let system = test.system();
    assert_eq!(system.create(&a(), &CounterState::total(1)).await.unwrap(), CreateActorResult::Created);
    match system.create(&a(), &CounterState::total(2)).await.unwrap() {
        CreateActorResult::Exists(record) => assert_eq!(record.state.unwrap()["total"], 1),
        other => panic!("{other:?}"),
    }
    assert_eq!(system.create_actor(&parse("actor://nothing/here"), None).await.unwrap(), CreateActorResult::NoSuchType);
    let bad = system.create_actor(&Counter::new("b").address(), Some(json!({"total": "x"}))).await.unwrap();
    assert!(matches!(bad, CreateActorResult::InvalidState(_)), "{bad:?}");
    // A stateless type takes no state.
    let echo = parse("actor://echo/e");
    assert!(matches!(system.create_actor(&echo, Some(json!({}))).await.unwrap(), CreateActorResult::InvalidState(_)));
    assert_eq!(system.create_actor(&echo, Some(Value::Null)).await.unwrap(), CreateActorResult::Created);
    // A state type that cannot be decoded from null needs a state on a hosting node.
    let c = system.create_actor(&Counter::new("c").address(), None).await.unwrap();
    assert!(matches!(c, CreateActorResult::InvalidState(_)), "{c:?}");
}

#[tokio::test(start_paused = true)]
async fn get_list_delete() {
    let test = harness().await;
    let system = test.system();
    test.create(&a(), &CounterState::total(3)).await;
    test.create(&Counter::new("b"), &CounterState::default()).await;
    let snapshot = system.get_actor(&a().address()).await.unwrap().unwrap();
    assert_eq!(snapshot.template, template::<Counter>());
    assert_eq!(snapshot.params, BTreeMap::from([("id".to_owned(), "a".to_owned())]));
    assert_eq!(snapshot.version, 0);
    assert_eq!(
        system.list_actors("actor://counter/", None, 100).await.unwrap().items,
        vec![a().address(), Counter::new("b").address()]
    );
    let pending = test.send_with(&a(), CounterMsg::Add { n: 1 }, SendOptions::new().delay(Duration::from_secs(300))).await;
    assert_eq!(system.delete_actor(&a().address()).await.unwrap(), DeleteResult::Deleted { purged: 1 });
    assert_eq!(test.outcome(&pending).await.unwrap().status, OutcomeStatus::Purged);
    assert_eq!(system.delete_actor(&a().address()).await.unwrap(), DeleteResult::Absent);
    assert!(system.get_actor(&a().address()).await.unwrap().is_none());
    assert!(system.pending(&a().address(), None, 100).await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn send_accepted_duplicate_no_such_instance_unknown_type_invalid_payload() {
    let test = harness().await;
    let system = test.system();
    test.create(&a(), &CounterState::default()).await;
    let addr = a().address();
    let id = MessageId::random();
    let with_id = || SendOptions::new().id(id.clone());
    let accepted = match system.send_message(&addr, "Add", json!({"n": 1}), with_id()).await.unwrap() {
        SendResult::Accepted(m) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(accepted.id, id);
    assert_eq!(
        system.send_message(&addr, "Add", json!({"n": 1}), with_id()).await.unwrap(),
        SendResult::Duplicate(Some(accepted))
    );
    assert_eq!(
        system.send_message(&Counter::new("zz").address(), "Add", json!({"n": 1}), SendOptions::new()).await.unwrap(),
        SendResult::NoSuchInstance
    );
    assert_eq!(
        system.send_message(&parse("actor://x/y"), "Add", json!({}), SendOptions::new()).await.unwrap(),
        SendResult::NoSuchInstance
    );
    assert_eq!(
        system.send_message(&addr, "Nope", json!({}), SendOptions::new()).await.unwrap(),
        SendResult::UnknownType { message_type: "Nope".into(), template: template::<Counter>() }
    );
    let invalid = system.send_message(&addr, "Add", json!({"n": "x"}), SendOptions::new()).await.unwrap();
    assert!(matches!(invalid, SendResult::InvalidPayload { .. }), "{invalid:?}");
    assert_eq!(test.pending(&a()).await.len(), 1);
    // Still a duplicate after it was handled (retained outcome).
    test.run_until_idle().await;
    let again = system.send_message(&addr, "Add", json!({"n": 1}), with_id()).await.unwrap();
    assert!(matches!(again, SendResult::Duplicate(_)), "{again:?}");
}

#[tokio::test(start_paused = true)]
async fn status_cancel_failures_types() {
    let test = TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap();
    let system = test.system();
    test.create(&a(), &CounterState::default()).await;
    let later = test.send_with(&a(), CounterMsg::Add { n: 1 }, SendOptions::new().delay(Duration::from_secs(60))).await;
    assert!(matches!(system.status(&later.id).await.unwrap(), Some(MessageStatus::Pending(_))));
    assert!(matches!(system.cancel(&later.id).await.unwrap(), CancelResult::Cancelled(_)));
    assert!(matches!(system.cancel(&later.id).await.unwrap(), CancelResult::AlreadyFinished(_)));
    assert_eq!(system.cancel(&MessageId::random()).await.unwrap(), CancelResult::NotFound);
    match system.status(&later.id).await.unwrap() {
        Some(MessageStatus::Finished(o)) => assert_eq!(o.status, OutcomeStatus::Cancelled),
        other => panic!("{other:?}"),
    }

    let boom = test.send(&a(), CounterMsg::Boom { reason: "x".into() }).await;
    test.run_until_idle().await;
    let failures = system.failures("actor://counter/", None, None, 100).await.unwrap().items;
    assert_eq!(failures.into_iter().map(|f| f.message_id).collect::<Vec<_>>(), vec![boom.id]);
    assert!(system.failures("actor://other/", None, None, 100).await.unwrap().items.is_empty());

    let types = system.types().await.unwrap();
    assert_eq!(types.iter().map(|t| t.template().clone()).collect::<Vec<_>>(), vec![template::<Counter>()]);
    let expected = Contract::new(
        template::<Counter>(),
        Some(schema::root_schema::<CounterState>()),
        schema::message_schemas::<CounterMsg>().unwrap(),
    );
    assert_eq!(*types[0].contract, expected);
    let resolved = system.type_of(&a().address()).await.unwrap().unwrap();
    assert_eq!(resolved.params, BTreeMap::from([("id".to_owned(), "a".to_owned())]));
    assert!(system.type_of(&parse("actor://x/y")).await.unwrap().is_none());
    assert!(system.health().await);
}

#[tokio::test]
async fn a_node_that_does_not_host_the_type_accepts_any_payload_and_the_hosting_node_applies_the_unhandled_policy() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::with_clock(Arc::new(VirtualClock::default())));
    let host = ActorSystem::builder(store.clone()).host(CounterActor).build();
    host.initialize().await.unwrap();
    let send_only = ActorSystem::builder(store.clone()).build();
    let addr = a().address();
    assert_eq!(send_only.create(&a(), &CounterState::default()).await.unwrap(), CreateActorResult::Created);
    // The type name is still checked through the registry ...
    assert_eq!(
        send_only.send_message(&addr, "Nope", json!({}), SendOptions::new()).await.unwrap(),
        SendResult::UnknownType { message_type: "Nope".into(), template: template::<Counter>() }
    );
    // ... but the payload is not.
    let bad = accepted(send_only.send_message(&addr, "Add", json!({"n": "x"}), SendOptions::new()).await.unwrap());
    let refused = host.send_message(&addr, "Add", json!({"n": "x"}), SendOptions::new()).await.unwrap();
    assert!(matches!(refused, SendResult::InvalidPayload { .. }), "{refused:?}");
    // A send-only node also accepts a state it cannot decode (it does not know the state type).
    assert_eq!(
        send_only.create_actor(&Counter::new("b").address(), Some(json!({"total": "x"}))).await.unwrap(),
        CreateActorResult::Created
    );
    // Typed sends work from any node.
    accepted(send_only.send(&a(), CounterMsg::Add { n: 2 }).await.unwrap());
    assert_eq!(send_only.run_due_turns(10).await.unwrap(), 0);
    assert_eq!(host.run_due_turns(1).await.unwrap(), 1);
    let failed = store.outcome(&bad.id).await.unwrap().unwrap();
    assert_eq!(failed.status, OutcomeStatus::Failed);
    assert!(failed.error.unwrap().starts_with("Payload of 'Add' cannot be decoded"));
    host.run_due_turns(1).await.unwrap();
    assert_eq!(state_of::<CounterState>(&*store, &addr).await.total, 2);
}

#[tokio::test(start_paused = true)]
async fn delivery_time_is_store_time_and_a_past_instant_is_clamped() {
    let test = TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap();
    test.create(&a(), &CounterState::default()).await;
    let now = test.now();
    let mut options = SendOptions::new();
    options.delivery = Delivery::At(time::sub(now, Duration::from_secs(300)));
    let past = accepted(test.system().send_message(&a().address(), "Add", json!({"n": 1}), options).await.unwrap());
    assert_eq!(past.deliver_at, now);
    let at = time::add(now, Duration::from_secs(300)).unwrap();
    let future = test.send_with(&a(), CounterMsg::Add { n: 1 }, SendOptions::new().at(at)).await;
    assert_eq!(future.deliver_at, at);
}
