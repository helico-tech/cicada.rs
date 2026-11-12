//! The in-process operations behind the HTTP endpoints.

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
async fn send_accepted_duplicate_no_such_instance_no_such_type_unknown_type_invalid_payload() {
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
        SendResult::NoSuchType
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

/// Scenario: A repeated send after the type was removed. The scenarios of an actor whose type is not registered are in
/// the SQLite upgrade tests, as only an upgrade leaves one.
#[tokio::test(start_paused = true)]
async fn a_repeated_send_stays_a_duplicate_after_the_actor_was_deleted_and_its_type_removed() {
    let test = TestActors::builder().with(|n| n.host(CounterActor)).build().await.unwrap();
    let system = test.system();
    test.create(&a(), &CounterState::default()).await;
    let id = MessageId::random();
    let with_id = || SendOptions::new().id(id.clone());
    assert!(matches!(
        system.send_message(&a().address(), "Add", json!({"n": 1}), with_id()).await.unwrap(),
        SendResult::Accepted(_)
    ));
    test.run_until_idle().await;

    assert!(matches!(system.delete_actor(&a().address()).await.unwrap(), DeleteResult::Deleted { .. }));
    assert_eq!(system.remove_type(&type_name::<Counter>()).await.unwrap(), RemoveTypeResult::Removed);
    let repeated = system.send_message(&a().address(), "Add", json!({"n": 1}), with_id()).await.unwrap();
    assert!(matches!(repeated, SendResult::Duplicate(Some(ref m)) if m.id == id), "{repeated:?}");
    assert_eq!(
        system.send_message(&a().address(), "Add", json!({"n": 1}), SendOptions::new()).await.unwrap(),
        SendResult::NoSuchType
    );
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
        schema::root_schema::<CounterState>(),
        schema::message_schemas::<CounterMsg>().unwrap(),
    );
    assert_eq!(*types[0].contract, expected);
    assert!(system.health().await);
}

#[tokio::test]
async fn a_node_that_does_not_host_the_type_accepts_any_payload_and_the_hosting_node_fails_an_undecodable_one() {
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

/// What a schema check was called with: `(schema, value)` per call.
#[derive(Default)]
struct Calls(std::sync::Mutex<Vec<(Value, Value)>>);

impl Calls {
    /// A check that records its call and answers `matches`.
    fn check(&self, matches: bool) -> impl Fn(&Value, &Value) -> bool + Send + Sync {
        move |schema: &Value, value: &Value| {
            self.0.lock().unwrap().push((schema.clone(), value.clone()));
            matches
        }
    }

    fn all(&self) -> Vec<(Value, Value)> {
        self.0.lock().unwrap().clone()
    }
}

/// The sentence of a refusal by the schema of a payload of `Add` to a counter, and of a state of a counter.
const ADD_REFUSED: &str = "Payload of 'Add' does not match the registered schema; GET /types/counter has it";
const STATE_REFUSED: &str = "State does not match the registered schema; GET /types/counter has it";

#[tokio::test(start_paused = true)]
async fn the_check_gets_the_schema_of_the_message_type_or_the_state_and_the_value() {
    let test = harness().await;
    let system = test.system();
    test.create(&a(), &CounterState::default()).await;
    let calls = Calls::default();
    let sent = system
        .send_message_checked(&a().address(), "Add", json!({"n": 1}), SendOptions::new(), &calls.check(true))
        .await
        .unwrap();
    accepted(sent);
    let add = schema::message_schemas::<CounterMsg>().unwrap().remove("Add").unwrap();
    assert_eq!(calls.all(), vec![(add, json!({"n": 1}))]);

    let calls = Calls::default();
    let b = Counter::new("b").address();
    let created = system.create_actor_checked(&b, json!({"total": 2}), &calls.check(true)).await.unwrap();
    assert_eq!(created, CreateActorResult::Created);
    assert_eq!(calls.all(), vec![(schema::root_schema::<CounterState>(), json!({"total": 2}))]);
}

/// A refusal names no place; a value that the decoder takes gets the sentence alone, on either node.
#[tokio::test(start_paused = true)]
async fn a_refusal_of_the_check_stores_nothing_on_a_node_that_hosts_the_type_or_not() {
    let test = harness().await;
    let send_only = ActorSystem::builder(test.store().clone()).build();
    test.create(&a(), &CounterState::default()).await;
    for system in [test.system(), &send_only] {
        let calls = Calls::default();
        let refused = system
            .send_message_checked(&a().address(), "Add", json!({"n": 1}), SendOptions::new(), &calls.check(false))
            .await
            .unwrap();
        assert_eq!(refused, SendResult::InvalidPayload { message_type: "Add".into(), error: ADD_REFUSED.into() });
        assert!(test.pending(&a()).await.is_empty());

        let b = Counter::new("b").address();
        let refused = system.create_actor_checked(&b, json!({"total": 2}), &calls.check(false)).await;
        assert_eq!(refused.unwrap(), CreateActorResult::InvalidState(STATE_REFUSED.into()));
        assert_eq!(system.get_actor(&b).await.unwrap(), None);
        assert_eq!(calls.all().len(), 2);
    }
}

#[tokio::test(start_paused = true)]
async fn the_check_is_not_called_for_a_repeated_id_an_unknown_type_an_unknown_message_type_or_a_null_state() {
    let test = harness().await;
    let system = test.system();
    test.create(&a(), &CounterState::default()).await;
    let calls = Calls::default();
    let check = calls.check(false);
    let (to, id) = (a().address(), MessageId::random());
    let first = system.send_message(&to, "Add", json!({"n": 1}), SendOptions::new().id(id.clone())).await;
    let first = accepted(first.unwrap());
    // A repeat whose payload the check would refuse is answered as a repeat.
    let repeated = system.send_message_checked(&to, "Add", json!({"n": "x"}), SendOptions::new().id(id), &check);
    assert_eq!(repeated.await.unwrap(), SendResult::Duplicate(Some(first)));
    let unknown = system.send_message_checked(&parse("x/y"), "Add", json!({}), SendOptions::new(), &check).await;
    assert_eq!(unknown.unwrap(), SendResult::NoSuchType);
    let nope = system.send_message_checked(&to, "Nope", json!({}), SendOptions::new(), &check).await;
    assert!(matches!(nope.unwrap(), SendResult::UnknownType { .. }));
    assert_eq!(
        system.create_actor_checked(&parse("x/y"), json!({}), &check).await.unwrap(),
        CreateActorResult::NoSuchType
    );
    let null = system.create_actor_checked(&Counter::new("n").address(), Value::Null, &check).await.unwrap();
    assert!(matches!(null, CreateActorResult::InvalidState(ref e) if e.contains("never null")), "{null:?}");
    assert!(calls.all().is_empty(), "{:?}", calls.all());
}

/// On a node that hosts the type the check runs first; when it refuses, nothing else runs and the detail is its sentence,
/// as on every node; when it accepts, the decoder answers.
#[tokio::test(start_paused = true)]
async fn on_a_node_that_hosts_the_type_the_check_runs_before_the_decoder_which_a_refusal_skips() {
    let test = harness().await;
    let system = test.system();
    let send_only = ActorSystem::builder(test.store().clone()).build();
    test.create(&a(), &CounterState::default()).await;
    let (refuse, accept) = (Calls::default(), Calls::default());
    let (refuse, accept) = (refuse.check(false), accept.check(true));
    let to = a().address();
    let error = |result: SendResult| match result {
        SendResult::InvalidPayload { error, .. } => error,
        other => panic!("{other:?}"),
    };
    let send = |system: &ActorSystem, check| {
        let system = system.clone();
        let to = to.clone();
        async move { system.send_message_checked(&to, "Add", json!({"n": "x"}), SendOptions::new(), check).await }
    };
    assert_eq!(error(send(system, &refuse).await.unwrap()), ADD_REFUSED);
    assert_eq!(error(send(&send_only, &refuse).await.unwrap()), ADD_REFUSED);
    assert!(error(send(system, &accept).await.unwrap()).starts_with("Payload of 'Add' cannot be decoded"));

    let b = Counter::new("b").address();
    let state_error = |result: CreateActorResult| match result {
        CreateActorResult::InvalidState(e) => e,
        other => panic!("{other:?}"),
    };
    let refused = state_error(system.create_actor_checked(&b, json!({"total": "x"}), &refuse).await.unwrap());
    assert_eq!(refused, STATE_REFUSED);
    let alone = state_error(send_only.create_actor_checked(&b, json!({"total": "x"}), &refuse).await.unwrap());
    assert_eq!(alone, STATE_REFUSED);
    let decoder = state_error(system.create_actor_checked(&b, json!({"total": "x"}), &accept).await.unwrap());
    assert!(decoder.starts_with("State cannot be decoded"), "{decoder}");
    assert!(test.pending(&a()).await.is_empty());
}

/// http-schema-check, Scenarios: A very long value; A very long value that only the decoder refuses (the cut of the
/// core). A refusal by the check quotes nothing of the value; the decoder of a hosting node quotes a text where it wants
/// a number, and its detail is cut.
#[tokio::test(start_paused = true)]
async fn the_detail_of_a_refusal_quotes_nothing_or_is_cut_at_its_limit_with_a_mark() {
    let test = harness().await;
    let system = test.system();
    test.create(&a(), &CounterState::default()).await;
    let long = json!("é".repeat(200_000));
    let (refuse, accept) = (|_: &Value, _: &Value| false, |_: &Value, _: &Value| true);
    let to = a().address();
    let sent = system.send_message_checked(&to, "Add", json!({"n": long}), SendOptions::new(), &refuse).await;
    assert_eq!(sent.unwrap(), SendResult::InvalidPayload { message_type: "Add".into(), error: ADD_REFUSED.into() });
    let sent = system.send_message_checked(&to, "Add", json!({"n": long}), SendOptions::new(), &accept).await;
    let SendResult::InvalidPayload { error, .. } = sent.unwrap() else { panic!("refused") };
    assert_eq!(error.chars().count(), crate::runtime::MAX_REFUSAL_DETAIL);
    assert!(error.starts_with("Payload of 'Add' cannot be decoded: ") && error.ends_with(" … (cut)"), "{error:.120}");

    let b = Counter::new("b").address();
    let created = system.create_actor_checked(&b, json!({"total": long}), &refuse).await;
    assert_eq!(created.unwrap(), CreateActorResult::InvalidState(STATE_REFUSED.into()));
    let created = system.create_actor_checked(&b, json!({"total": long}), &accept).await;
    let CreateActorResult::InvalidState(error) = created.unwrap() else { panic!("refused") };
    assert_eq!(error.chars().count(), crate::runtime::MAX_REFUSAL_DETAIL);
    assert!(error.starts_with("State cannot be decoded: ") && error.ends_with(" … (cut)"), "{error:.120}");
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
        SendResult::NoSuchType
    );
    let def = Contract::new(
        name.clone(),
        schema::root_schema::<CounterState>(),
        schema::message_schemas::<CounterMsg>().unwrap(),
    )
    .to_type_def();
    system.store().register(def).await.unwrap();
    assert_eq!(system.create_actor(&address, json!({"total": 0})).await.unwrap(), CreateActorResult::Created);
}
