//! The schema check at the HTTP door (http-schema-check, ADR 0026): a node that hosts the types and one that does not,
//! on one store, on virtual time.

mod common;

use std::borrow::Cow;
use std::time::Duration;

use axum::Router;
use cicada::api::problem_types as pt;
use cicada::prelude::*;
use cicada::retention::Retention;
use cicada::time::Timestamp;
use cicada_axum::actor_api;
use common::*;
use schemars::{Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// type `form`: messages that deny unknown fields or carry an instant, and a state whose schema the validator cannot
/// use.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Form {
    id: String,
}

impl ActorRef for Form {
    const TYPE: &'static str = "form";
    type Message = FormMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
enum FormMsg {
    Strict { n: i32 },
    At { at: Timestamp },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FormState {
    #[serde(default)]
    code: String,
}

/// A schema with a pattern that the validator's regular expressions do not know (a look-behind).
impl JsonSchema for FormState {
    fn schema_name() -> Cow<'static, str> {
        "FormState".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "object", "properties": {"code": {"type": "string", "pattern": "(?<=a)b"}}})
    }
}

struct FormActor;

impl Actor for FormActor {
    type Ref = Form;
    type State = FormState;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: FormMsg) -> HandlerResult {
        Ok(())
    }
}

/// The routers of a node that hosts `counter` and `form` and of one that does not, on one store, with `counter/a`.
struct Nodes {
    host: Router,
    other: Router,
    other_system: ActorSystem,
}

async fn nodes() -> Nodes {
    let store = memory_store();
    let host = actor_api(node_with(store.clone(), |b| b.host(CounterActor).host(FormActor)).await);
    let other_system = node_with(store, |b| b).await;
    let other = actor_api(other_system.clone());
    assert_eq!(put(&host, "/actors/counter/a", r#"{"state": {}}"#).await.status, 201);
    Nodes { host, other, other_system }
}

const SEND: &str = "/actors/counter/a/messages";

fn detail(reply: &Reply) -> String {
    reply.json()["detail"].as_str().expect("a detail").to_owned()
}

/// Waits until the message `id` has the status `status`, as `GET /messages/{id}` tells.
async fn wait_for(app: &Router, id: &str, status: &str) {
    eventually(async || get_json(app, &format!("/messages/{id}")).await["status"] == status).await;
}

/// Scenarios: A node that does not host the type; A node that hosts the type.
#[tokio::test(start_paused = true)]
async fn a_payload_that_does_not_match_is_refused_by_every_node_with_the_same_detail_and_not_stored() {
    let n = nodes().await;
    let other = post(&n.other, SEND, r#"{"id": "m-other", "type": "Add", "payload": {"n": "x"}}"#).await;
    assert_problem(&other, 422, pt::INVALID_PAYLOAD);
    assert_eq!(detail(&other), "Payload of 'Add' does not match the schema: at /n: want integer, but got string");
    assert_problem(&get(&n.other, "/messages/m-other").await, 404, pt::NO_SUCH_MESSAGE);

    let host = post(&n.host, SEND, r#"{"id": "m-host", "type": "Add", "payload": {"n": "x"}}"#).await;
    assert_problem(&host, 422, pt::INVALID_PAYLOAD);
    assert_eq!(detail(&host), detail(&other));
    assert_problem(&get(&n.host, "/messages/m-host").await, 404, pt::NO_SUCH_MESSAGE);
}

/// Scenario: A payload that matches.
#[tokio::test(start_paused = true)]
async fn a_payload_that_matches_is_accepted_by_a_node_that_does_not_host_the_type() {
    let n = nodes().await;
    let id = send_id(&n.other, SEND, r#"{"type": "Add", "payload": {"n": 1}}"#).await;
    wait_for(&n.other, &id, "done").await;
}

/// Scenario: A field that the type does not know.
#[tokio::test(start_paused = true)]
async fn a_field_that_the_type_does_not_know_is_refused() {
    let n = nodes().await;
    put(&n.host, "/actors/form/f", r#"{"state": {}}"#).await;
    for app in [&n.other, &n.host] {
        let reply =
            post(app, "/actors/form/f/messages", r#"{"type": "Strict", "payload": {"n": 1, "extra": true}}"#).await;
        assert_problem(&reply, 422, pt::INVALID_PAYLOAD);
        assert!(detail(&reply).contains("'extra' not allowed"), "{}", detail(&reply));
    }
}

/// Scenario: A required field is missing.
#[tokio::test(start_paused = true)]
async fn a_missing_required_field_is_refused_and_named() {
    let n = nodes().await;
    let reply = post(&n.other, SEND, r#"{"type": "Add", "payload": {}}"#).await;
    assert_problem(&reply, 422, pt::INVALID_PAYLOAD);
    assert_eq!(detail(&reply), "Payload of 'Add' does not match the schema: at the top level: missing properties 'n'");
}

/// Scenario: A message type the contract does not have.
#[tokio::test(start_paused = true)]
async fn a_message_type_the_contract_does_not_have_is_an_unknown_message_type_as_before() {
    let n = nodes().await;
    let reply = post(&n.other, SEND, r#"{"type": "Nope", "payload": {"n": "x"}}"#).await;
    assert_problem(&reply, 422, pt::UNKNOWN_MESSAGE_TYPE);
}

/// Scenarios of a create: A node that does not host the type; A state that matches; The actor exists.
#[tokio::test(start_paused = true)]
async fn a_state_that_does_not_match_is_refused_by_a_node_that_does_not_host_the_type_also_for_an_existing_actor() {
    let n = nodes().await;
    let refused = put(&n.other, "/actors/counter/b", r#"{"state": {"total": "x"}}"#).await;
    assert_problem(&refused, 422, pt::INVALID_STATE);
    assert_eq!(detail(&refused), "State does not match the schema: at /total: want integer, but got string");
    assert_problem(&get(&n.other, "/actors/counter/b").await, 404, pt::NO_SUCH_INSTANCE);

    assert_eq!(put(&n.other, "/actors/counter/b", r#"{"state": {"total": 1}}"#).await.status, 201);

    let existing = put(&n.other, "/actors/counter/a", r#"{"state": {"total": "x"}}"#).await;
    assert_problem(&existing, 422, pt::INVALID_STATE);
    let actor = get_json(&n.other, "/actors/counter/a").await;
    assert_eq!((actor["state"].clone(), actor["version"].clone()), (json!({}), json!(0)));
}

/// Scenario: A repeat of a message that does not match.
#[tokio::test(start_paused = true)]
async fn a_repeat_of_a_pending_message_that_does_not_match_is_answered_as_a_repeat() {
    let n = nodes().await;
    let (to, id) = (ActorAddress::parse("counter/a").unwrap(), MessageId::new("m1").unwrap());
    let options = SendOptions::new().id(id).delay(Duration::from_secs(3600));
    let sent = n.other_system.send_message(&to, "Add", json!({"n": "x"}), options).await.unwrap();
    assert!(matches!(sent, cicada::SendResult::Accepted(_)), "{sent:?}");
    let repeat = post(&n.other, SEND, r#"{"id": "m1", "type": "Add", "payload": {"n": "x"}, "delay": "PT1H"}"#).await;
    assert_eq!(repeat.status, 200, "{}", repeat.text);
}

/// Scenario: A repeat of a message whose outcome is hidden.
#[tokio::test(start_paused = true)]
async fn a_repeat_of_a_message_whose_outcome_retention_hides_is_checked() {
    let n = nodes().await;
    let (to, id) = (ActorAddress::parse("counter/a").unwrap(), MessageId::new("m2").unwrap());
    let options = SendOptions::new().id(id).retention(Retention::REMOVE_ON_FAILED);
    let sent = n.other_system.send_message(&to, "Add", json!({"n": "x"}), options).await.unwrap();
    assert!(matches!(sent, cicada::SendResult::Accepted(_)), "{sent:?}");
    eventually(async || get(&n.other, "/messages/m2").await.status == 404).await;
    let repeat = post(&n.other, SEND, r#"{"id": "m2", "type": "Add", "payload": {"n": "x"}}"#).await;
    assert_problem(&repeat, 422, pt::INVALID_PAYLOAD);
}

/// Scenario: A send without an id.
#[tokio::test(start_paused = true)]
async fn a_send_without_an_id_that_does_not_match_is_refused() {
    let n = nodes().await;
    assert_problem(&post(&n.other, SEND, r#"{"type": "Add", "payload": {"n": "x"}}"#).await, 422, pt::INVALID_PAYLOAD);
}

/// Scenarios: A payload that the schema allows and the type cannot decode; The same payload on a node that does not host
/// the type. The schema of an `i32` has no bounds.
#[tokio::test(start_paused = true)]
async fn a_payload_that_only_the_schema_allows_is_refused_by_the_hosting_node_and_fails_in_its_turn_otherwise() {
    let n = nodes().await;
    let too_large = r#"{"type": "Add", "payload": {"n": 3000000000}}"#;
    let host = post(&n.host, SEND, too_large).await;
    assert_problem(&host, 422, pt::INVALID_PAYLOAD);
    assert!(detail(&host).starts_with("Payload of 'Add' cannot be decoded: "), "{}", detail(&host));

    let id = send_id(&n.other, SEND, too_large).await;
    wait_for(&n.other, &id, "failed").await;
}

/// Scenario: A state that the schema allows and the type cannot decode.
#[tokio::test(start_paused = true)]
async fn a_state_that_only_the_schema_allows_is_refused_by_the_hosting_node() {
    let n = nodes().await;
    let reply = put(&n.host, "/actors/counter/c", r#"{"state": {"total": 3000000000}}"#).await;
    assert_problem(&reply, 422, pt::INVALID_STATE);
    assert!(detail(&reply).contains("cannot be decoded"), "{}", detail(&reply));
    assert_problem(&get(&n.host, "/actors/counter/c").await, 404, pt::NO_SUCH_INSTANCE);
}

/// Scenario: A date that is not a date.
#[tokio::test(start_paused = true)]
async fn a_date_that_is_not_a_date_passes_the_door_and_fails_in_its_turn() {
    let n = nodes().await;
    put(&n.host, "/actors/form/f", r#"{"state": {}}"#).await;
    let id = send_id(&n.other, "/actors/form/f/messages", r#"{"type": "At", "payload": {"at": "tomorrow"}}"#).await;
    wait_for(&n.other, &id, "failed").await;
}

/// Scenario: A state schema that cannot be used.
#[tokio::test(start_paused = true)]
async fn a_state_schema_that_cannot_be_used_is_not_enforced() {
    let n = nodes().await;
    assert_eq!(put(&n.other, "/actors/form/g", r#"{"state": {"code": 5}}"#).await.status, 201);
    let state: Value = get_json(&n.other, "/actors/form/g").await["state"].clone();
    assert_eq!(state, json!({"code": 5}));
}
