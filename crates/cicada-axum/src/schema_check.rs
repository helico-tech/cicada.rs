//! The check of a payload or a state against its registered JSON Schema at the HTTP door (ADR 0026), and the sample
//! checks that test a type against both its schema and its decoder.
//!
//! The validator is `jsonschema`, on draft 2020-12 for the root of a schema. It asserts no `format`, whatever draft a
//! schema names, and loads nothing from outside the schema document: no file and no network. A schema it cannot compile
//! is not enforced at the door, and a warning names it once in the life of the process; the sample checks fail on it.
//! The door asks only whether a value matches (`is_valid`) and names no place: finding a place can cost time and memory
//! without bound for a type with several shapes (decision 8). The sample checks, which run on a developer's own
//! samples, name the first place. Every value is checked on a thread of its own with a large stack, as the validator
//! recurses per level of a type that refers to itself (decision 11).

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use jsonschema::{Draft, Retrieve, Uri, ValidationError, Validator};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// What the validator says of a sample.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// The value matches the schema.
    Matches,
    /// It does not: the first place that does not match, with its path in the value and the reason.
    DoesNotMatch(String),
    /// The validator cannot compile the schema; why.
    Unusable(String),
}

/// A retriever that refuses every address: the check uses nothing outside the schema document. The standard
/// metaschemas are built into the validator and never reach a retriever.
struct NoRetriever;

impl Retrieve for NoRetriever {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("{uri} is outside the schema, and the check loads nothing").into())
    }
}

/// The stack of the thread that checks a value (decision 11). The stack the validator needs per level is set by the
/// steps of the schema between two levels of a type that refers to itself; the JSON parser of a request lets through a
/// payload or a state of 126 levels. It is address space: only the pages the check touches are used.
const CHECK_STACK: usize = 64 << 20;

/// Runs `work` on a scoped thread of its own with [`CHECK_STACK`] of stack, so that no nesting the parser lets through
/// overflows the stack of a thread of the runtime, whatever the type. A thread that cannot be started fails the request
/// (500), as the check must not be skipped.
fn on_own_stack<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new().name("cicada-schema-check".to_owned()).stack_size(CHECK_STACK);
        let handle = thread.spawn_scoped(scope, work).expect("a thread to check a value");
        handle.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// The validator of `schema`: draft 2020-12, no format asserted, nothing loaded. `Err` with why it cannot be used.
fn validator(schema: &Value) -> Result<Validator, String> {
    let options =
        jsonschema::options().with_draft(Draft::Draft202012).should_validate_formats(false).with_retriever(NoRetriever);
    // One line: a reason may hold line breaks.
    options.build(schema).map_err(|e| e.to_string().split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Whether `value` matches `schema`, on a stack of its own; `Err` with why the schema cannot be used. Only `is_valid`:
/// it builds no error, so its cost is linear in the value whatever the type.
fn matches(schema: &Value, value: &Value) -> Result<bool, String> {
    on_own_stack(|| validator(schema).map(|validator| validator.is_valid(value)))
}

/// The check of the door: true when `value` matches `schema`. It names no place (decision 8). A schema that cannot be
/// used lets `value` through; the first time, a warning names it with `what` ("message type 'Add' of counter").
pub(crate) fn check(schema: &Value, value: &Value, what: &str) -> bool {
    matches(schema, value).unwrap_or_else(|why| {
        warn_once(schema, what, &why);
        true
    })
}

/// Logs that the schema of `what` cannot be used, once for each schema text in the life of the process; true when it
/// logged.
fn warn_once(schema: &Value, what: &str, why: &str) -> bool {
    static WARNED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);
    let first = WARNED.lock().unwrap_or_else(|e| e.into_inner()).insert(cicada::contract::canonical_json(schema));
    if first {
        tracing::warn!("The schema of {what} cannot be used by the validator and is not enforced at the door: {why}");
    }
    first
}

/// The check of a sample, on a stack of its own: whether it matches and, when it does not, the first place. Finding the
/// place can take long for a deep value of a type with several shapes; a sample is a developer's own, in a test.
fn verdict(schema: &Value, value: &Value) -> Verdict {
    on_own_stack(|| {
        let validator = match validator(schema) {
            Ok(validator) => validator,
            Err(why) => return Verdict::Unusable(why),
        };
        if validator.is_valid(value) {
            return Verdict::Matches;
        }
        match validator.validate(value) {
            Ok(()) => Verdict::Matches,
            Err(error) => Verdict::DoesNotMatch(place(&error)),
        }
    })
}

/// The place of a mismatch: "at /path: reason", "the top level" for the value itself. It names no address of the schema
/// and no path into it.
fn place(error: &ValidationError) -> String {
    let path = error.instance_path().to_string();
    let at = if path.is_empty() { "the top level" } else { &path };
    format!("at {at}: {error}")
}

/// Panics unless `sample` (`{"type": "Add", ...}`, a message of the family `M` in its own serde form) passes the
/// registered schema of its message type and decodes as `M`, as the HTTP door and a node that hosts the type check
/// it. The panic says which of the two refused the sample and why, and for the schema the first place that does not
/// match, which the door does not name; finding it can take long for a deep value of a type with several shapes (an
/// enum with data, an `Option` of a struct). A schema that the validator cannot use fails too.
///
/// Feed it what the clients of a type send over HTTP, and what the type itself serialises:
///
/// ```
/// # use schemars::JsonSchema;
/// # use serde::{Deserialize, Serialize};
/// #[derive(Serialize, Deserialize, JsonSchema)]
/// #[serde(tag = "type")]
/// enum CounterMsg {
///     Add { n: i32 },
/// }
///
/// cicada_axum::assert_message_sample::<CounterMsg>(&serde_json::json!({"type": "Add", "n": 1}));
/// cicada_axum::assert_message_sample::<CounterMsg>(&serde_json::to_value(CounterMsg::Add { n: 2 }).unwrap());
/// ```
pub fn assert_message_sample<M: JsonSchema + DeserializeOwned>(sample: &Value) {
    let family = std::any::type_name::<M>();
    let Some(message_type) = sample.get(cicada::message::TYPE_TAG).and_then(Value::as_str) else {
        panic!("A sample of {family} is an object with its type name in \"type\": {sample}");
    };
    let schemas = cicada::schema::message_schemas::<M>().unwrap_or_else(|e| panic!("{e}"));
    let Some(schema) = schemas.get(message_type) else {
        panic!("{family} has no message type '{message_type}'");
    };
    let mut payload = sample.clone();
    if let Some(fields) = payload.as_object_mut() {
        fields.remove(cicada::message::TYPE_TAG);
    }
    passes_schema(schema, &payload, &format!("the sample of '{message_type}'"));
    if let Err(e) = cicada::message::decode::<M>(message_type, &payload) {
        panic!("The decoder refused the sample of '{message_type}': {e}");
    }
}

/// Panics unless `sample` passes the state schema of `S` and decodes as `S`, as the HTTP door and a node that hosts
/// the type check the state of a create. The panic says which of the two refused the sample and why, and for the schema
/// the first place that does not match, which the door does not name; finding it can take long for a deep value of a
/// type with several shapes. A schema that the validator cannot use fails too.
///
/// ```
/// # use schemars::JsonSchema;
/// # use serde::{Deserialize, Serialize};
/// #[derive(Serialize, Deserialize, JsonSchema)]
/// struct CounterState {
///     total: i32,
/// }
///
/// cicada_axum::assert_state_sample::<CounterState>(&serde_json::json!({"total": 3}));
/// ```
pub fn assert_state_sample<S: JsonSchema + DeserializeOwned>(sample: &Value) {
    let state = std::any::type_name::<S>();
    passes_schema(&cicada::schema::root_schema::<S>(), sample, &format!("the state sample of {state}"));
    if let Err(e) = serde_json::from_value::<S>(sample.clone()) {
        panic!("The decoder refused the state sample of {state}: {e}");
    }
}

fn passes_schema(schema: &Value, sample: &Value, what: &str) {
    match verdict(schema, sample) {
        Verdict::Matches => {}
        Verdict::DoesNotMatch(place) => panic!("The schema refused {what}: {place}"),
        Verdict::Unusable(why) => panic!("The schema of {what} cannot be used by the validator: {why}"),
    }
}

/// Types with several schema steps between two of their levels, shared with the test of the HTTP door.
#[cfg(test)]
#[path = "../tests/common/shapes.rs"]
mod shapes;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::shapes::{Chain, PutMsg, Shape, Stack, Wide, build, payload};
    use super::*;

    const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

    fn object(properties: Value) -> Value {
        json!({"$schema": DRAFT, "type": "object", "properties": properties})
    }

    fn refused(schema: &Value, value: &Value) -> String {
        match verdict(schema, value) {
            Verdict::DoesNotMatch(place) => place,
            other => panic!("{other:?}"),
        }
    }

    fn unusable(schema: &Value) -> String {
        match verdict(schema, &json!({"n": 1})) {
            Verdict::Unusable(why) => why,
            other => panic!("{other:?}"),
        }
    }

    /// The sample check, not the door: of two places that do not match it names the first, with what was expected.
    #[test]
    fn the_sample_check_names_the_first_of_two_places_with_what_was_expected() {
        let schema = object(json!({"n": {"type": "integer"}, "name": {"type": "string"}}));
        assert_eq!(refused(&schema, &json!({"n": "x", "name": 5})), r#"at /n: "x" is not of type "integer""#);
    }

    /// The sample check, not the door: for a field inside an optional part it names the part, with a general reason.
    #[test]
    fn the_sample_check_names_an_optional_part_for_a_field_inside_it() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Address {
            street: String,
        }
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Ship {
            address: Option<Address>,
        }
        let schema = cicada::schema::root_schema::<Ship>();
        let place = refused(&schema, &json!({"address": {"street": 7}}));
        assert_eq!(
            place,
            r#"at /address: {"street":7} is not valid under any of the schemas listed in the 'anyOf' keyword"#
        );
        assert!(!place.contains("$defs") && !place.contains('#'), "no address of the schema: {place}");
    }

    #[test]
    fn a_value_that_matches_passes_and_a_missing_field_and_the_top_level_are_named() {
        let mut schema = object(json!({"n": {"type": "integer"}}));
        schema["required"] = json!(["n"]);
        assert_eq!(verdict(&schema, &json!({"n": 1})), Verdict::Matches);
        assert_eq!(refused(&schema, &json!({})), r#"at the top level: "n" is a required property"#);
        assert_eq!(refused(&schema, &json!([1])), r#"at the top level: [1] is not of type "object""#);
    }

    /// Scenario: A schema that refers to a file, and to a port, relatively and under an `$id`: nothing is loaded, and
    /// the schema cannot be used. The file exists and the port listens; either would refuse every object.
    #[test]
    fn a_schema_that_refers_outside_itself_is_not_used_and_nothing_is_loaded() {
        let dir = std::env::temp_dir().join(format!("cicada-schema-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("refuse.json");
        std::fs::write(&file, r#"{"type": "string"}"#).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let refs = [
            json!({"$schema": DRAFT, "$ref": format!("file://{}", file.display())}),
            json!({"$schema": DRAFT, "$ref": format!("http://127.0.0.1:{port}/refuse.json")}),
            json!({"$schema": DRAFT, "$ref": "refuse.json"}),
            json!({"$schema": DRAFT, "$id": format!("http://127.0.0.1:{port}/root.json"), "$ref": "refuse.json"}),
        ];
        for schema in refs {
            let why = unusable(&schema);
            assert!(why.contains("not present in a registry"), "{why}");
            assert!(check(&schema, &json!({"n": 1}), "message type 'Read' of files"));
        }
        let accepted = listener.accept();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(accepted, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock), "a connection came in");
    }

    /// Scenario: A schema with a pattern the validator does not know.
    #[test]
    fn a_schema_with_a_pattern_the_validator_does_not_know_is_not_enforced_and_warned_about_once() {
        let schema = object(json!({"code": {"type": "string", "pattern": "["}}));
        assert!(unusable(&schema).contains("is not a \"regex\""));
        assert!(check(&schema, &json!({"code": 5}), "message type 'Code' of codes"));
        assert!(!warn_once(&schema, "message type 'Code' of codes", "again"), "warned once already");
        let other = object(json!({"other": {"type": "string", "pattern": "["}}));
        assert!(warn_once(&other, "message type 'Other' of codes", "a schema of its own"));
    }

    /// Formats are notes whatever draft the schema names, and a schema is read as draft 2020-12 whatever its root
    /// names.
    #[test]
    fn a_date_that_is_not_a_date_passes_as_no_format_is_asserted_whatever_the_draft() {
        for draft in [DRAFT, "http://json-schema.org/draft-07/schema#", "http://json-schema.org/draft-04/schema#"] {
            let schema = json!({"$schema": draft, "type": "object", "properties": {
                "at": {"type": "string", "format": "date-time"}, "mail": {"format": "email"}}});
            assert_eq!(verdict(&schema, &json!({"at": "tomorrow", "mail": "nobody"})), Verdict::Matches, "{draft}");
        }
    }

    /// The sample check, not the door: of a value with many mismatches, deep in a type that refers to itself, it names
    /// the first, quickly. The time and memory of a refusal at the door are tested in `tests/memory.rs`.
    #[test]
    fn a_deep_value_with_many_mismatches_names_the_first_quickly() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Node {
            items: Vec<i32>,
            next: Option<Box<Node>>,
        }
        let mut value = json!({"items": vec!["x"; 2_000], "next": null});
        for _ in 1..100 {
            value = json!({"items": vec!["x"; 2_000], "next": value});
        }
        let schema = cicada::schema::root_schema::<Node>();
        let start = std::time::Instant::now();
        let place = refused(&schema, &value);
        let took = start.elapsed();
        assert_eq!(place, r#"at /items/0: "x" is not of type "integer""#);
        assert!(took < std::time::Duration::from_secs(30), "the check took {took:?}");
    }

    /// Scenarios: A value at the deepest level the parser allows; A type with several schema steps between its levels
    /// (decision 11). Three such types, at 21 and 24 levels (where, with the first validator, a check on a default stack
    /// ended the process in a test build) and at 126, the deepest payload or state a request body holds; as a state and
    /// as a payload, matching and not. Through the check of the door and that of the sample checks, on a thread with the
    /// default stack of 2 MiB.
    #[test]
    fn a_value_is_checked_on_a_stack_of_its_own_whatever_its_type_and_depth() {
        fn shape<T: Shape>() {
            let state = cicada::schema::root_schema::<T>();
            let put = cicada::schema::message_schemas::<PutMsg<T>>().unwrap().remove("Put").unwrap();
            for levels in [21, 24, 126] {
                for ok in [true, false] {
                    for (schema, value) in [(&state, build::<T>(levels, ok)), (&put, payload::<T>(levels, ok))] {
                        assert_eq!(check(schema, &value, T::NAME), ok, "the door: {} at {levels} levels", T::NAME);
                        match verdict(schema, &value) {
                            Verdict::Matches => assert!(ok, "{} at {levels} levels matches", T::NAME),
                            Verdict::DoesNotMatch(place) => assert!(!ok, "{} at {levels} levels: {place}", T::NAME),
                            other => panic!("{} at {levels} levels: {other:?}", T::NAME),
                        }
                    }
                }
            }
        }
        let run = || {
            shape::<Chain>();
            shape::<Stack>();
            shape::<Wide>();
        };
        std::thread::Builder::new().stack_size(2 << 20).spawn(run).unwrap().join().unwrap();
    }
}
