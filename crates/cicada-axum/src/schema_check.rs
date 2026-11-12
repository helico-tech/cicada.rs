//! The check of a payload or a state against its registered JSON Schema at the HTTP door (ADR 0026), and the sample
//! checks that test a type against both its schema and its decoder.
//!
//! The validator is `boon`, on draft 2020-12. It asserts no `format`, and loads nothing from outside the schema
//! document: no file and no network. A schema it cannot compile is not enforced at the door, and a warning names it once
//! in the life of the process; the sample checks fail on it. A value nested deeper than [`SHALLOW`] levels is checked on
//! a thread of its own with a larger stack, as the validator recurses per level.

use std::collections::HashSet;
use std::error::Error;
use std::sync::{LazyLock, Mutex};

use boon::{CompileError, Compiler, Draft, ErrorKind, InstanceToken, Schemas, UrlLoader, ValidationError};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The most places that the detail of a refusal names.
const MOST_PLACES: usize = 5;

/// The address under which a schema is compiled. Not a file, so that a relative reference resolves to nothing.
const ROOT: &str = "urn:cicada:schema";

/// What the validator says of a value.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// The value matches the schema.
    Matches,
    /// It does not: up to five places, each with its path in the value and the reason, and whether there are more.
    DoesNotMatch(String),
    /// The validator cannot compile the schema; why.
    Unusable(String),
}

/// A loader that refuses every address: the check uses nothing outside the schema document. The standard metaschemas
/// are built into `boon` and never reach a loader.
struct NoLoader;

impl UrlLoader for NoLoader {
    fn load(&self, url: &str) -> Result<Value, Box<dyn Error>> {
        Err(format!("{url} is outside the schema, and the check loads nothing").into())
    }
}

/// The deepest value that is checked on the calling thread (decision 11). The validator recurses per level of a type
/// that refers to itself, at about 42 KiB of stack a level in a test build and 11 KiB in a release build, so 24 levels
/// take half of a default stack of 2 MiB in a test build.
const SHALLOW: usize = 24;

/// The stack of the thread of its own that checks a deeper value. The JSON parser of a request lets through 127 levels,
/// the body included, so a payload or a state of at most 126; they take about 5.3 MiB in a test build, and this is six
/// times that. Only the pages the check touches are used.
const DEEP_STACK: usize = 32 << 20;

/// Checks `value` against `schema`; a value deeper than [`SHALLOW`] on a thread of its own with [`DEEP_STACK`] of stack,
/// so that no nesting the parser lets through overflows the stack of a thread of the runtime. Nothing is skipped.
fn verdict(schema: &Value, value: &Value) -> Verdict {
    if depth(value) <= SHALLOW {
        return verdict_here(schema, value);
    }
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new().name("cicada-deep-check".to_owned()).stack_size(DEEP_STACK);
        // A thread that cannot be started fails the request (500), as the check must not be skipped.
        let handle =
            thread.spawn_scoped(scope, || verdict_here(schema, value)).expect("a thread to check a deep value");
        handle.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// The nesting of `value`: 0 for a scalar, 1 for an object or an array of scalars. Without recursion, for any depth.
fn depth(value: &Value) -> usize {
    let mut deepest = 0;
    let mut open = vec![(value, 1)];
    while let Some((value, level)) = open.pop() {
        let inner: Box<dyn Iterator<Item = &Value>> = match value {
            Value::Object(fields) => Box::new(fields.values()),
            Value::Array(items) => Box::new(items.iter()),
            _ => continue,
        };
        deepest = deepest.max(level);
        open.extend(inner.filter(|v| v.is_object() || v.is_array()).map(|v| (v, level + 1)));
    }
    deepest
}

/// Checks `value` against `schema` on the calling thread.
fn verdict_here(schema: &Value, value: &Value) -> Verdict {
    let mut compiler = Compiler::new();
    compiler.set_default_draft(Draft::V2020_12);
    compiler.use_loader(Box::new(NoLoader));
    let mut schemas = Schemas::new();
    // One line: the alternate form adds the cause, over several lines.
    let unusable =
        |e: CompileError| Verdict::Unusable(format!("{e:#}").split_whitespace().collect::<Vec<_>>().join(" "));
    if let Err(e) = compiler.add_resource(ROOT, schema.clone()) {
        return unusable(e);
    }
    let index = match compiler.compile(ROOT, &mut schemas) {
        Ok(index) => index,
        Err(e) => return unusable(e),
    };
    match schemas.validate(value, index) {
        Ok(()) => Verdict::Matches,
        Err(error) => Verdict::DoesNotMatch(places(&error)),
    }
}

/// The leaves of the validator's tree of errors, in its order: at most five, each "at /path: reason", then "and 3 more"
/// when there are more. A leaf names no address of the schema and no path into it. Under an `anyOf` or a `oneOf` (an
/// enum with data, an `Option`), the alternatives that the value cannot be come last (see [`names_another`]), so the
/// reasons of the variant that the value names come first.
fn places(error: &ValidationError) -> String {
    fn leaves<'e, 's, 'v>(error: &'e ValidationError<'s, 'v>, found: &mut Vec<&'e ValidationError<'s, 'v>>) {
        if error.causes.is_empty() {
            found.push(error);
        }
        let mut causes: Vec<_> = error.causes.iter().collect();
        if matches!(error.kind, ErrorKind::AnyOf | ErrorKind::OneOf(_)) {
            let part = error.instance_location.tokens.len();
            causes.sort_by_key(|alternative| names_another(alternative, part)); // stable: the validator's order stays
        }
        causes.into_iter().for_each(|cause| leaves(cause, found));
    }
    let mut found = Vec::new();
    leaves(error, &mut found);
    let mut text: Vec<String> = found
        .iter()
        .take(MOST_PLACES)
        .map(|leaf| {
            let path = leaf.instance_location.to_string();
            let at = if path.is_empty() { "the top level".to_owned() } else { path };
            let why = match &leaf.kind {
                ErrorKind::Schema { .. } | ErrorKind::RefCycle { .. } => "does not match".to_owned(),
                kind => kind.to_string(),
            };
            format!("at {at}: {why}")
        })
        .collect();
    if found.len() > MOST_PLACES {
        text.push(format!("and {} more", found.len() - MOST_PLACES));
    }
    text.join("; ")
}

/// True when the errors of an `anyOf` or `oneOf` alternative for the part `part` tokens deep show that the value is
/// another variant: its tag names another one (a `const` or `enum` on a property of the part fails, as for an
/// internally or adjacently tagged enum), the part is not of the alternative's kind (a unit variant's text, the `null`
/// of an `Option`), or it holds a property that the alternative does not allow (the key of an externally tagged enum).
///
/// It looks only at the alternative's own error and its direct causes, never deeper, and builds no path: every error is
/// looked at by at most its two nearest groups, so the walk over the errors stays linear. A child's location extends
/// its parent's, so a location one token longer than the part's that ends in a property is a property of the part.
fn names_another(alternative: &ValidationError, part: usize) -> bool {
    std::iter::once(alternative).chain(&alternative.causes).any(|error| {
        let tokens = &error.instance_location.tokens;
        match error.kind {
            ErrorKind::Const { .. } | ErrorKind::Enum { .. } => {
                tokens.len() == part + 1 && matches!(tokens.last(), Some(InstanceToken::Prop(_)))
            }
            ErrorKind::Type { .. } | ErrorKind::AdditionalProperties { .. } => tokens.len() == part,
            _ => false,
        }
    })
}

/// The check of the door: `Err` with the places of `value` that do not match `schema`. A schema that cannot be used
/// lets `value` through; the first time, a warning names it with `what` ("message type 'Add' of counter").
pub(crate) fn check(schema: &Value, value: &Value, what: &str) -> Result<(), String> {
    match verdict(schema, value) {
        Verdict::Matches => Ok(()),
        Verdict::DoesNotMatch(places) => Err(places),
        Verdict::Unusable(why) => {
            warn_once(schema, what, &why);
            Ok(())
        }
    }
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

/// Panics unless `sample` (`{"type": "Add", ...}`, a message of the family `M` in its own serde form) passes the
/// registered schema of its message type and decodes as `M`, as the HTTP door and a node that hosts the type check
/// it. The panic says which of the two refused the sample and why; a schema that the validator cannot use fails too.
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
/// the type check the state of a create. The panic says which of the two refused the sample and why; a schema that
/// the validator cannot use fails too.
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
        Verdict::DoesNotMatch(places) => panic!("The schema refused {what}: {places}"),
        Verdict::Unusable(why) => panic!("The schema of {what} cannot be used by the validator: {why}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(properties: Value) -> Value {
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "properties": properties})
    }

    fn refused(schema: &Value, value: &Value) -> String {
        match verdict(schema, value) {
            Verdict::DoesNotMatch(places) => places,
            other => panic!("{other:?}"),
        }
    }

    /// Scenario: Two places that do not match.
    #[test]
    fn two_places_that_do_not_match_are_both_named_with_what_was_expected() {
        let schema = object(json!({"n": {"type": "integer"}, "name": {"type": "string"}}));
        let places = refused(&schema, &json!({"n": "x", "name": 5}));
        assert_eq!(places, "at /n: want integer, but got string; at /name: want string, but got number");
    }

    /// Scenario: A field inside an optional part.
    #[test]
    fn a_field_inside_an_optional_part_is_named_by_its_path() {
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
        let places = refused(&schema, &json!({"address": {"street": 7}}));
        assert!(places.contains("at /address/street: want string, but got number"), "{places}");
        assert!(!places.contains("urn:") && !places.contains("$defs") && !places.contains('#'), "{places}");
    }

    /// Scenario: More than five places.
    #[test]
    fn more_than_five_places_name_five_and_say_that_there_are_more() {
        let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let schema = object(names.iter().map(|n| (n.to_string(), json!({"type": "integer"}))).collect());
        let value: Value = names.iter().map(|n| (n.to_string(), json!("x"))).collect::<serde_json::Map<_, _>>().into();
        let places = refused(&schema, &value);
        assert_eq!(places.matches("want integer, but got string").count(), 5, "{places}");
        assert!(places.ends_with("; and 3 more"), "{places}");
    }

    #[test]
    fn a_value_that_matches_passes_and_a_missing_field_is_named() {
        let schema = object(json!({"n": {"type": "integer"}}));
        let mut schema_with_required = schema.clone();
        schema_with_required["required"] = json!(["n"]);
        assert_eq!(verdict(&schema_with_required, &json!({"n": 1})), Verdict::Matches);
        assert_eq!(refused(&schema_with_required, &json!({})), "at the top level: missing properties 'n'");
    }

    /// Scenario: A schema that refers to a file. The file exists and would refuse every object.
    #[test]
    fn a_schema_that_refers_to_a_file_is_not_used_and_the_file_is_not_read() {
        let dir = std::env::temp_dir().join(format!("cicada-schema-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("refuse.json");
        std::fs::write(&file, r#"{"type": "string"}"#).unwrap();
        let url = format!("file://{}", file.display());
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$ref": url});
        let verdict = verdict(&schema, &json!({"n": 1}));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(verdict, Verdict::Unusable(ref why) if why.contains("loads nothing")), "{verdict:?}");
        assert_eq!(check(&schema, &json!({"n": 1}), "message type 'Read' of files"), Ok(()));
    }

    /// Scenario: A schema with a pattern the validator does not know.
    #[test]
    fn a_schema_with_a_pattern_the_validator_does_not_know_is_not_enforced_and_warned_about_once() {
        let schema = object(json!({"code": {"type": "string", "pattern": "(?<=a)b"}}));
        assert!(matches!(verdict(&schema, &json!({"code": 5})), Verdict::Unusable(_)));
        assert_eq!(check(&schema, &json!({"code": 5}), "message type 'Code' of codes"), Ok(()));
        assert!(!warn_once(&schema, "message type 'Code' of codes", "again"), "warned once already");
        let other = object(json!({"other": {"type": "string", "pattern": "(?<=a)b"}}));
        assert!(warn_once(&other, "message type 'Other' of codes", "a schema of its own"));
    }

    /// The reasons of the variant that the value names come first, before those of the variants it cannot be.
    #[test]
    fn the_reasons_of_the_variant_that_the_value_names_come_first() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[serde(tag = "kind")]
        #[allow(dead_code)]
        enum Shape {
            Circle { r: f64 },
            Square { side: f64 },
            Triangle { a: f64, b: f64 },
            Hexagon { s: f64 },
            Dot,
        }
        #[derive(serde::Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase")]
        #[allow(dead_code)]
        enum Outer {
            Dot,
            Circle { r: f64 },
            Square { side: f64 },
        }
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Draw {
            shape: Shape,
            outer: Option<Outer>,
        }
        let schema = cicada::schema::root_schema::<Draw>();
        let places = refused(&schema, &json!({"shape": {"kind": "Hexagon", "s": "x"}}));
        assert!(places.starts_with("at /shape/s: want number, but got string; "), "{places}");
        assert!(places.ends_with("and 3 more"), "the other variants follow: {places}");
        let places = refused(&schema, &json!({"shape": {"kind": "Triangle", "a": "x"}}));
        assert!(places.starts_with("at /shape: missing properties 'b'; at /shape/a: want number"), "{places}");
        let places = refused(&schema, &json!({"shape": {"kind": "Dot"}, "outer": {"square": {"side": "x"}}}));
        assert!(places.starts_with("at /outer/square/side: want number, but got string; "), "{places}");
    }

    /// The walk over the errors is linear: a value that refers to itself 100 deep through an `Option`, with 2,000 wrong
    /// items a level (200,000 errors under 100 nested `anyOf`s), is checked in about the time of its validation, some
    /// 0.2 s in a test build. A walk over every subtree at every level took 5.6 s for it in a release build.
    #[test]
    fn a_deep_value_with_many_mismatches_is_checked_in_linear_time() {
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
        let places = refused(&schema, &value);
        let took = start.elapsed();
        assert!(places.starts_with("at /items/0: want integer, but got string; "), "{places}");
        assert!(places.ends_with("; and 200094 more"), "{places}"); // and the `null` of 99 levels
        assert!(took < std::time::Duration::from_secs(30), "the check took {took:?}");
    }

    /// The depth of a value, without recursion.
    #[test]
    fn the_depth_of_a_value_counts_its_containers() {
        assert_eq!(depth(&json!(1)), 0);
        assert_eq!(depth(&json!({})), 1);
        assert_eq!(depth(&json!({"a": [1, {"b": []}], "c": "x"})), 4);
        let mut deep = Value::Null;
        for _ in 0..100_000 {
            deep = Value::Array(vec![deep]); // not `json!`, which would copy `deep` by recursion
        }
        assert_eq!(depth(&deep), 100_000);
        std::mem::forget(deep); // dropping it would recurse 100,000 levels
    }

    /// Scenario: A value at the deepest level the parser allows (decision 11), on an ordinary test thread: 126 levels of a
    /// type that refers to itself, the deepest payload or state that a request body of 127 levels holds.
    #[test]
    fn a_value_at_the_deepest_level_the_parser_allows_is_checked_on_a_stack_of_its_own() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Node {
            items: Vec<i32>,
            next: Option<Box<Node>>,
        }
        let schema = cicada::schema::root_schema::<Node>();
        for (item, matches) in [(json!(1), true), (json!("x"), false)] {
            let mut value = json!({"items": [item.clone()], "next": null}); // two levels: the object and its items
            for _ in 2..126 {
                value = json!({"items": [item.clone()], "next": value});
            }
            assert_eq!(depth(&value), 126);
            match verdict(&schema, &value) {
                Verdict::Matches => assert!(matches),
                Verdict::DoesNotMatch(places) => {
                    assert!(!matches, "{places}");
                    assert!(places.starts_with("at /items/0: want integer, but got string; "), "{places}");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_date_that_is_not_a_date_passes_as_no_format_is_asserted() {
        let schema = object(json!({"at": {"type": "string", "format": "date-time"}, "mail": {"format": "email"}}));
        assert_eq!(verdict(&schema, &json!({"at": "tomorrow", "mail": "nobody"})), Verdict::Matches);
    }
}
