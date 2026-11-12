//! The sample checks (http-schema-check): a sample passes the schema and the decoder, or the panic says which refused it.

use std::borrow::Cow;

use cicada::store::MessageId;
use cicada::{ActorAddress, Failed, StopCause, Stopped};
use cicada_axum::{assert_message_sample, assert_state_sample};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum PayMsg {
    Pay {
        #[serde(alias = "sum")]
        amount: i32,
    },
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Wallet {
    balance: i32,
}

/// A text whose schema has a pattern that the validator does not know (a look-behind).
#[derive(Debug, Serialize, Deserialize)]
struct Code(String);

impl JsonSchema for Code {
    fn schema_name() -> Cow<'static, str> {
        "Code".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "string", "pattern": "(?<=a)b"})
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CodeMsg {
    Set { code: Code },
}

/// Scenario: A sample that passes both.
#[test]
fn a_message_that_the_type_serialised_passes() {
    assert_message_sample::<PayMsg>(&serde_json::to_value(PayMsg::Pay { amount: 3 }).unwrap());
    assert_message_sample::<PayMsg>(&json!({"type": "Pay", "amount": 3}));
}

/// Scenario: A sample that only the decoder accepts.
#[test]
#[should_panic(expected = "The schema refused the sample of 'Pay': at the top level: missing properties 'amount'")]
fn a_message_that_names_a_field_by_its_alias_fails_on_the_schema() {
    assert_message_sample::<PayMsg>(&json!({"type": "Pay", "sum": 3}));
}

/// Scenario: A sample that only the schema accepts.
#[test]
#[should_panic(expected = "The decoder refused the sample of 'Pay': Payload of 'Pay' cannot be decoded")]
fn a_message_with_a_number_too_large_for_its_field_fails_on_the_decoder() {
    assert_message_sample::<PayMsg>(&json!({"type": "Pay", "amount": 3_000_000_000_u64}));
}

/// Scenario: A state sample (the first part).
#[test]
fn a_state_that_the_type_serialised_passes() {
    assert_state_sample::<Wallet>(&serde_json::to_value(Wallet { balance: 3 }).unwrap());
}

/// Scenario: A state sample (the second part).
#[test]
#[should_panic(expected = "The schema refused the state sample of samples::Wallet: at /balance: want integer")]
fn a_state_with_a_field_of_the_wrong_kind_fails() {
    assert_state_sample::<Wallet>(&json!({"balance": "x"}));
}

/// Scenario: A schema that cannot be used.
#[test]
#[should_panic(expected = "The schema of the sample of 'Set' cannot be used by the validator")]
fn a_sample_of_a_type_whose_schema_cannot_be_compiled_fails() {
    assert_message_sample::<CodeMsg>(&json!({"type": "Set", "code": "b"}));
}

#[test]
#[should_panic(expected = "has no message type 'Nope'")]
fn a_sample_of_a_message_type_the_family_does_not_have_fails() {
    assert_message_sample::<PayMsg>(&json!({"type": "Nope"}));
}

/// The family of a watcher, with the two notifications the core writes.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum WatcherMsg {
    Failed(Failed),
    Stopped(Stopped),
}

#[test]
fn the_notifications_failed_and_stopped_pass_in_a_watchers_family() {
    let address = |text: &str| ActorAddress::parse(text).unwrap();
    let failed = |correlation_id: Option<&str>, from: Option<ActorAddress>| Failed {
        actor: address("counter/a"),
        message_id: MessageId::new("m-1").unwrap(),
        message_type: "Add".into(),
        payload: json!({"n": "x"}),
        correlation_id: correlation_id.map(str::to_owned),
        from,
        error: "Payload of 'Add' cannot be decoded: invalid type".into(),
        attempts: 1,
    };
    let mut samples = vec![
        WatcherMsg::Failed(failed(None, None)),
        WatcherMsg::Failed(failed(Some("c-1"), Some(address("orders/o:1")))),
    ];
    for (cause, by) in [(StopCause::Stopped, Some(address("counter/a"))), (StopCause::Deleted, None)] {
        samples.push(WatcherMsg::Stopped(Stopped { actor: address("counter/a"), cause, by }));
    }
    samples.push(WatcherMsg::Stopped(Stopped { actor: address("tree/leaf"), cause: StopCause::Parent, by: None }));
    for sample in samples {
        assert_message_sample::<WatcherMsg>(&serde_json::to_value(sample).unwrap());
    }
}
