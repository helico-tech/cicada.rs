//! The in-process operations behind the HTTP endpoints.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use super::common::*;
use crate::ActorSystem;
use crate::address::ActorAddress;
use crate::contract::Contract;
use crate::runtime::{CreateActorResult, SendResult};
use crate::schema;
use crate::store::{
    Backend, CancelResult, DeleteResult, Delivery, MessageId, MessageStatus, OutcomeStatus, RemoveTypeResult, Store,
};
use crate::testing::{TestActors, VirtualClock};
use crate::time;
use crate::turn::SendOptions;
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
        CreateActorResult::Exists(record) => assert_eq!(record.state["total"], 1),
        other => panic!("{other:?}"),
    }
    assert_eq!(system.create_actor(&parse("nothing/here"), json!({})).await.unwrap(), CreateActorResult::NoSuchType);
    let bad = system.create_actor(&Counter::new("b").address(), json!({"total": "x"})).await.unwrap();
    assert!(matches!(bad, CreateActorResult::InvalidState(_)), "{bad:?}");
    // A type that keeps no data (NoState) refuses a state with a field.
    let echo = parse("echo/e");
    let oops = system.create_actor(&echo, json!({"oops": 1})).await.unwrap();
    assert!(matches!(oops, CreateActorResult::InvalidState(_)), "{oops:?}");
    assert_eq!(system.create_actor(&echo, json!({})).await.unwrap(), CreateActorResult::Created);
}

#[tokio::test(start_paused = true)]
async fn a_create_with_a_null_state_is_refused() {
    let test = harness().await;
    let system = test.system();
    for address in [Counter::new("c").address(), parse("echo/e")] {
        match system.create_actor(&address, Value::Null).await.unwrap() {
            CreateActorResult::InvalidState(e) => assert!(e.contains("never null"), "{e}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(system.get_actor(&address).await.unwrap(), None);
    }
}

#[tokio::test(start_paused = true)]
async fn get_list_delete() {
    let test = harness().await;
    let system = test.system();
    test.create(&a(), &CounterState::total(3)).await;
    test.create(&Counter::new("b"), &CounterState::default()).await;
    let snapshot = system.get_actor(&a().address()).await.unwrap().unwrap();
    assert_eq!(snapshot.address.type_name(), &type_name::<Counter>());
    assert_eq!(snapshot.params, BTreeMap::from([("id".to_owned(), "a".to_owned())]));
    assert_eq!(snapshot.version, 0);
    assert_eq!(
        system.list_actors("counter/", None, 100).await.unwrap().items,
        vec![a().address(), Counter::new("b").address()]
    );
    let pending =
        test.send_with(&a(), CounterMsg::Add { n: 1 }, SendOptions::new().delay(Duration::from_secs(300))).await;
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
        system.send_message(&parse("x/y"), "Add", json!({}), SendOptions::new()).await.unwrap(),
        SendResult::NoSuchInstance
    );
    assert_eq!(
        system.send_message(&addr, "Nope", json!({}), SendOptions::new()).await.unwrap(),
        SendResult::UnknownType { message_type: "Nope".into(), type_name: type_name::<Counter>() }
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
async fn status_cancel_outcomes_types() {
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

    // Later than the cancel, so that the listing's order does not depend on the random message ids.
    test.advance_time_by(Duration::from_secs(1)).await;
    let boom = test.send(&a(), CounterMsg::Boom { reason: "x".into() }).await;
    test.run_until_idle().await;
    let failed = Some(OutcomeStatus::Failed);
    let failures = system.outcomes(failed, "counter/", None, None, 100).await.unwrap().items;
    assert_eq!(failures.into_iter().map(|f| f.message_id).collect::<Vec<_>>(), vec![boom.id.clone()]);
    assert!(system.outcomes(failed, "other/", None, None, 100).await.unwrap().items.is_empty());
    // Without a status, every finished message is listed: the cancelled one and the failed one.
    let all = system.outcomes(None, "counter/", None, None, 100).await.unwrap().items;
    assert_eq!(
        all.into_iter().map(|o| (o.message_id, o.status)).collect::<Vec<_>>(),
        vec![(later.id.clone(), OutcomeStatus::Cancelled), (boom.id.clone(), OutcomeStatus::Failed)]
    );
    let cancelled = system.outcomes(Some(OutcomeStatus::Cancelled), "", None, None, 100).await.unwrap().items;
    assert_eq!(cancelled.into_iter().map(|o| o.message_id).collect::<Vec<_>>(), vec![later.id]);

    let types = system.types().await.unwrap();
    assert_eq!(types.iter().map(|t| t.name().clone()).collect::<Vec<_>>(), vec![type_name::<Counter>()]);
    let expected = Contract::new(
        type_name::<Counter>(),
        vec!["id".into()],
        schema::root_schema::<CounterState>(),
        schema::message_schemas::<CounterMsg>().unwrap(),
    );
    assert_eq!(*types[0].contract, expected);
    assert!(system.health().await);
}

#[tokio::test]
async fn a_node_that_does_not_host_the_type_accepts_any_payload_and_the_hosting_node_applies_the_unhandled_policy() {
    let store = Store::memory_on(Arc::new(VirtualClock::default()));
    let host = ActorSystem::builder(store.clone()).host(CounterActor).build();
    host.initialize().await.unwrap();
    let send_only = ActorSystem::builder(store.clone()).build();
    let addr = a().address();
    assert_eq!(send_only.create(&a(), &CounterState::default()).await.unwrap(), CreateActorResult::Created);
    // The type name is still checked through the registry ...
    assert_eq!(
        send_only.send_message(&addr, "Nope", json!({}), SendOptions::new()).await.unwrap(),
        SendResult::UnknownType { message_type: "Nope".into(), type_name: type_name::<Counter>() }
    );
    // ... but the payload is not.
    let bad = accepted(send_only.send_message(&addr, "Add", json!({"n": "x"}), SendOptions::new()).await.unwrap());
    let refused = host.send_message(&addr, "Add", json!({"n": "x"}), SendOptions::new()).await.unwrap();
    assert!(matches!(refused, SendResult::InvalidPayload { .. }), "{refused:?}");
    // A send-only node also accepts a state it cannot decode (it does not know the state type), but never null.
    assert_eq!(
        send_only.create_actor(&Counter::new("b").address(), json!({"total": "x"})).await.unwrap(),
        CreateActorResult::Created
    );
    let null = send_only.create_actor(&Counter::new("n").address(), Value::Null).await.unwrap();
    assert!(matches!(null, CreateActorResult::InvalidState(_)), "{null:?}");
    // Typed sends work from any node.
    accepted(send_only.send(&a(), CounterMsg::Add { n: 2 }).await.unwrap());
    assert_eq!(send_only.run_due_turns(10).await.unwrap(), 0);
    assert_eq!(host.run_due_turns(1).await.unwrap(), 1);
    let failed = outcome_of(&store, &bad.id).await.unwrap();
    assert_eq!(failed.status, OutcomeStatus::Failed);
    assert!(failed.error.unwrap().starts_with("Payload of 'Add' cannot be decoded"));
    host.run_due_turns(1).await.unwrap();
    assert_eq!(state_of::<CounterState>(&store, &addr).await.total, 2);
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

#[tokio::test(start_paused = true)]
async fn a_type_is_removed_only_while_no_actor_of_it_exists_and_is_not_served_afterwards() {
    let test = TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap();
    let system = test.system();
    let name = type_name::<Counter>();
    test.create(&a(), &CounterState::default()).await;

    assert_eq!(system.remove_type(&name).await.unwrap(), RemoveTypeResult::InUse);
    assert_eq!(system.types().await.unwrap().len(), 1, "an in-use type stays registered");
    assert_eq!(
        system.remove_type(&crate::address::TypeName::parse("nope").unwrap()).await.unwrap(),
        RemoveTypeResult::Absent
    );

    assert!(matches!(system.delete_actor(&a().address()).await.unwrap(), DeleteResult::Deleted { .. }));
    assert_eq!(system.remove_type(&name).await.unwrap(), RemoveTypeResult::Removed);
    assert!(system.types().await.unwrap().is_empty());
    assert_eq!(system.remove_type(&name).await.unwrap(), RemoveTypeResult::Absent);

    // The removed type is gone for the resolver too: no create and no send until it is registered again.
    let address = a().address();
    assert_eq!(system.create_actor(&address, json!({"total": 0})).await.unwrap(), CreateActorResult::NoSuchType);
    assert_eq!(
        system.send_message(&address, "Add", json!({"n": 1}), SendOptions::new()).await.unwrap(),
        SendResult::NoSuchInstance
    );
    let def = Contract::new(
        name.clone(),
        vec!["id".into()],
        schema::root_schema::<CounterState>(),
        schema::message_schemas::<CounterMsg>().unwrap(),
    )
    .to_type_def();
    system.store().register(def).await.unwrap();
    assert_eq!(system.create_actor(&address, json!({"total": 0})).await.unwrap(), CreateActorResult::Created);
}
