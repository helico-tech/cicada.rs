//! Canonical JSON, JSON Schemas and the message codec.

use std::collections::BTreeMap;

use cicada::contract::{canonical_json, canonical_sha256};
use cicada::message::{self, EncodedMessage, FamilyInfo};
use cicada::prelude::*;
use cicada::schema::{message_schemas, root_schema};
use cicada::testing::TestActors;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn parse(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

// ---- exact numbers ----

/// JSON text is parsed with exact rounding (serde_json's `float_roundtrip`, ADR 0043): a float that the default parser
/// reads one bit off is read back with its bits. The feature is set on the shared dependency of the workspace, which
/// every crate inherits, so a consumer of the crates gets it without a dev dependency of this workspace.
#[test]
fn json_text_is_parsed_with_exact_rounding_and_the_shared_dependency_sets_it() {
    for x in [1.0715660391465826e-75, -1.0715660391465826e-75, f64::from_bits(1), f64::MAX, -0.0] {
        let read: Value = serde_json::from_str(&serde_json::to_string(&json!(x)).unwrap()).unwrap();
        assert_eq!(read.as_f64().map(f64::to_bits), Some(x.to_bits()), "{x:e}");
    }
    let manifest = include_str!("../../../Cargo.toml");
    let line = manifest.lines().find(|l| l.starts_with("serde_json =")).expect("the shared dependency");
    assert!(line.contains("\"float_roundtrip\""), "{line}");
    assert!(include_str!("../Cargo.toml").contains("serde_json.workspace = true"), "cicada inherits it");
}

// ---- canonical JSON ----

#[test]
fn keys_are_sorted_recursively_and_whitespace_dropped() {
    assert_eq!(
        canonical_json(&parse(r#"{ "c": 1.5, "b": true, "a": [3, {"y": "é\"q", "x": null}] }"#)),
        r#"{"a":[3,{"x":null,"y":"é\"q"}],"b":true,"c":1.5}"#
    );
}

#[test]
fn hash_ignores_key_order_but_not_array_order() {
    let h1 = canonical_sha256(&parse(r#"{"a":1,"b":[1,2]}"#));
    let h2 = canonical_sha256(&parse(r#"{"b":[1,2],"a":1}"#));
    let h3 = canonical_sha256(&parse(r#"{"a":1,"b":[2,1]}"#));
    assert_eq!(h1, h2);
    assert_ne!(h1, h3);
    let hex = h1.strip_prefix("sha256:").unwrap();
    assert!(hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{h1}");
}

#[test]
fn known_hash() {
    // sha256 of the 2-byte string "{}"
    assert_eq!(
        canonical_sha256(&parse("{}")),
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
}

// ---- JSON Schemas ----

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
enum Priority {
    Low,
    High,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Shape {
    Circle { radius: f64 },
    Square { side: f64 },
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Order {
    sku: String,
    quantity: i32,
    note: Option<String>,
    #[serde(default = "low")]
    priority: Priority,
    #[serde(default)]
    tags: Vec<String>,
    prices: BTreeMap<String, i64>,
    shape: Shape,
    at: String,
    #[serde(default)]
    reply_to: Option<String>,
    #[serde(default)]
    extra: Option<Value>,
}

fn low() -> Priority {
    Priority::Low
}

#[test]
fn defaults_are_not_required_and_nullable_accepts_null() {
    let schema = root_schema::<Order>();
    let required: Vec<&str> = schema["required"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
    for optional in ["priority", "tags", "replyTo"] {
        assert!(!required.contains(&optional), "{optional} in {required:?}");
    }
    for needed in ["sku", "quantity", "prices", "shape", "at"] {
        assert!(required.contains(&needed), "{needed} not in {required:?}");
    }
    let note = schema["properties"]["note"].to_string();
    assert!(note.contains("null"), "{note}");
}

fn has_additional_properties_false(value: &Value) -> bool {
    match value {
        Value::Object(o) => o
            .iter()
            .any(|(k, v)| (k == "additionalProperties" && v == &json!(false)) || has_additional_properties_false(v)),
        Value::Array(a) => a.iter().any(has_additional_properties_false),
        _ => false,
    }
}

#[test]
fn no_additional_properties_false_anywhere_because_unknown_keys_are_ignored() {
    assert!(!has_additional_properties_false(&root_schema::<Order>()));
    assert!(!has_additional_properties_false(&root_schema::<Shape>()));
}

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CodecMsg {
    Ping,
    Draw {
        shape: Shape,
        #[serde(default = "one")]
        count: i32,
    },
    Inner(Inner),
}

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Inner {
    n: i32,
}

fn one() -> i32 {
    1
}

#[test]
fn a_message_without_fields_is_an_empty_object() {
    assert_eq!(
        message_schemas::<CodecMsg>().unwrap()["Ping"],
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "title": "Ping", "type": "object", "properties": {}})
    );
}

#[test]
fn primitive_and_collection_roots() {
    assert_eq!(root_schema::<String>()["type"], "string");
    assert_eq!(root_schema::<Vec<i32>>()["type"], "array");
    assert!(root_schema::<String>().get("properties").is_none());
}

#[test]
fn generation_is_deterministic() {
    assert_eq!(canonical_json(&root_schema::<Order>()), canonical_json(&root_schema::<Order>()));
    assert_eq!(message_schemas::<CodecMsg>().unwrap(), message_schemas::<CodecMsg>().unwrap());
}

// ---- message codec ----

#[test]
fn encode_uses_the_variant_fields_without_the_tag() {
    assert_eq!(
        message::encode(&CodecMsg::Ping).unwrap(),
        EncodedMessage { message_type: "Ping".into(), payload: json!({}) }
    );
    let draw = message::encode(&CodecMsg::Draw { shape: Shape::Circle { radius: 2.0 }, count: 1 }).unwrap();
    assert_eq!(draw.message_type, "Draw");
    assert_eq!(draw.payload, json!({"shape": {"type": "circle", "radius": 2.0}, "count": 1}));
}

#[test]
fn family_lists_its_type_names() {
    let family = FamilyInfo::of::<CodecMsg>().unwrap();
    assert_eq!(family.schemas.keys().collect::<Vec<_>>(), vec!["Draw", "Inner", "Ping"]);
}

#[test]
fn decode_round_trips_and_ignores_unknown_keys_and_a_stray_type_key() {
    let payload = json!({"shape": {"type": "square", "side": 1.0}, "extra": 1, "type": "Ping"});
    assert_eq!(
        message::decode::<CodecMsg>("Draw", &payload).unwrap(),
        CodecMsg::Draw { shape: Shape::Square { side: 1.0 }, count: 1 }
    );
    assert_eq!(message::decode::<CodecMsg>("Ping", &json!({})).unwrap(), CodecMsg::Ping);
    let inner = message::encode(&CodecMsg::Inner(Inner { n: 3 })).unwrap();
    assert_eq!(message::decode::<CodecMsg>("Inner", &inner.payload).unwrap(), CodecMsg::Inner(Inner { n: 3 }));
}

#[test]
fn decode_failures() {
    assert!(message::decode::<CodecMsg>("Nope", &json!({})).is_err());
    assert!(message::decode::<CodecMsg>("Draw", &json!([])).is_err());
    let e = message::decode::<CodecMsg>("Draw", &json!({"count": 2})).unwrap_err().to_string();
    assert!(e.starts_with("Payload of 'Draw' cannot be decoded"), "{e}");
    assert!(message::decode::<CodecMsg>("Draw", &json!({"shape": {"type": "circle", "radius": "x"}})).is_err());
}

#[test]
fn a_payload_that_does_not_decode_is_a_message_error() {
    let payload = json!({"shape": {"type": "circle", "radius": 1.0}, "count": "two"});
    let e: message::MessageError = message::decode::<CodecMsg>("Draw", &payload).unwrap_err();
    assert!(e.to_string().contains("'Draw'"), "{e}");
    assert_eq!(e.to_string().matches("cannot be decoded").count(), 1, "{e}");
    let e: anyhow::Error = message::encode(&Inner { n: 1 }).map_err(anyhow::Error::from).unwrap_err();
    assert!(e.to_string().contains("is not a message family"), "{e}");
}

#[test]
fn a_family_that_is_not_a_tagged_enum_is_rejected() {
    assert!(FamilyInfo::of::<Inner>().is_err());
    assert!(FamilyInfo::of::<Priority>().is_err());
    assert!(message::encode(&Inner { n: 1 }).is_err());
}

// ---- names that a reference escapes ----

/// A name with a space and `<>`, which a reference to its definition escapes.
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Line <v2>")]
struct Line {
    sku: String,
    priority: Priority,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum LineMsg {
    Put(Line),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Lines {
    id: String,
}

impl ActorRef for Lines {
    const TYPE: &'static str = "lines";
    type Message = LineMsg;
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct LinesState {
    skus: Vec<String>,
}

struct LinesActor;

impl Actor for LinesActor {
    type Ref = Lines;
    type State = LinesState;

    async fn handle(&self, turn: &mut Turn<Self>, message: LineMsg) -> HandlerResult {
        let LineMsg::Put(line) = message;
        turn.state_mut().skus.push(line.sku);
        Ok(())
    }
}

/// Scenario: A newtype variant of a renamed type. The node starts, which it did not before 0.15.0, and handles it.
#[tokio::test(start_paused = true)]
async fn a_node_starts_with_a_newtype_variant_of_a_renamed_type() {
    let test = TestActors::builder().with(|node| node.host(LinesActor)).build().await.expect("the node starts");
    let lines = Lines { id: "l".into() };
    test.create(&lines, &LinesState::default()).await;
    test.send(&lines, LineMsg::Put(Line { sku: "a".into(), priority: Priority::High })).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, LinesState>(&lines).await.skus, ["a"]);
}
