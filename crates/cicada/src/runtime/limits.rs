//! Limits on the envelope fields of a send and on a payload or a state. Within them every backend stores a message
//! without error; beyond them a send is refused before it reaches the store, the same way on every backend: from
//! outside an actor as a bad request (`400`), inside a turn with a [`crate::turn::TurnError::Invalid`] at the call. No
//! field, and no string or key of a payload or state, may contain a NUL character (PostgreSQL text and JSONB cannot
//! hold one), and a payload or a state holds at most [`MAX_DEPTH`] arrays and objects inside each other (ADR 0029).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::store::MessageId;

/// Longest message id.
pub const MAX_ID_LENGTH: usize = 200;
/// Longest message type name.
pub const MAX_TYPE_LENGTH: usize = 200;
/// Longest correlation id.
pub const MAX_CORRELATION_ID_LENGTH: usize = 1_000;
/// Most headers on one message.
pub const MAX_HEADERS: usize = 64;
/// Longest header name.
pub const MAX_HEADER_NAME_LENGTH: usize = 200;
/// Longest header value.
pub const MAX_HEADER_VALUE_LENGTH: usize = 4_000;
/// Deepest payload or state: the most arrays and objects inside each other. `{"a": {"b": 1}}` is 2 deep, a scalar 0.
/// A view of a value at this depth is at most 69 levels deep, which every JSON parser that allows 128 reads (ADR 0029).
pub const MAX_DEPTH: usize = 64;
/// Deepest state or message schema that [`ActorSystem::register_contract`](super::ActorSystem::register_contract)
/// accepts, counted as [`MAX_DEPTH`] counts a value. The stored contract and a type view add at most two levels and the
/// type list four, so every JSON parser that allows 128 reads them. Not a limit on values, nor on local hosting.
pub const MAX_SCHEMA_DEPTH: usize = 120;

/// Checks the fields of one send, naming the first field that is out of bounds. Lengths count characters.
pub(crate) fn check(
    id: &MessageId,
    message_type: &str,
    correlation_id: Option<&str>,
    headers: &BTreeMap<String, String>,
) -> Result<(), String> {
    field("id", id.as_str(), MAX_ID_LENGTH)?;
    field("type", message_type, MAX_TYPE_LENGTH)?;
    if let Some(c) = correlation_id {
        field("correlationId", c, MAX_CORRELATION_ID_LENGTH)?;
    }
    if headers.len() > MAX_HEADERS {
        return Err(format!("At most {MAX_HEADERS} headers are allowed, got {}", headers.len()));
    }
    for (name, value) in headers {
        check_header(name, value)?;
    }
    Ok(())
}

/// Checks the name and the value of one header, naming the one that is out of bounds.
pub(crate) fn check_header(name: &str, value: &str) -> Result<(), String> {
    field("header name", name, MAX_HEADER_NAME_LENGTH)?;
    field(&format!("header '{name}'"), value, MAX_HEADER_VALUE_LENGTH)
}

/// Checks a send's envelope and payload.
pub(crate) fn check_envelope(
    id: &MessageId,
    message_type: &str,
    payload: &Value,
    correlation_id: Option<&str>,
    headers: &BTreeMap<String, String>,
) -> Result<(), String> {
    check(id, message_type, correlation_id, headers)?;
    check_json("payload", Some(payload))
}

/// Checks that no string and no object key in `value` contains a NUL character, and that it is at most [`MAX_DEPTH`]
/// deep, naming the first problem it meets. One walk, which stops at the first level beyond the limit, so it refuses a
/// value of any depth without overflowing a stack.
pub(crate) fn check_json(what: &str, value: Option<&Value>) -> Result<(), String> {
    match value.map(|v| walk(v, 0, MAX_DEPTH, true)) {
        Some(Err(problem)) => Err(problem.text(what, MAX_DEPTH)),
        _ => Ok(()),
    }
}

/// Checks only that `value` is at most [`MAX_DEPTH`] deep: a stored payload that an operator retries was checked for a
/// NUL character when it was written.
pub(crate) fn check_depth(what: &str, value: &Value) -> Result<(), String> {
    walk(value, 0, MAX_DEPTH, false).map_err(|problem| problem.text(what, MAX_DEPTH))
}

/// Checks a contract that is registered without hosting: the state and each message schema is an object or a boolean,
/// at most [`MAX_SCHEMA_DEPTH`] deep and free of NUL characters, and each message name obeys the send limits of a type
/// name. Names the first problem it meets.
pub(crate) fn check_contract(contract: &crate::contract::Contract) -> Result<(), String> {
    check_schema("state schema", contract.state_schema())?;
    for (name, schema) in contract.message_schemas() {
        field("message name", name, MAX_TYPE_LENGTH)?;
        check_schema(&format!("schema of message '{name}'"), schema)?;
    }
    Ok(())
}

fn check_schema(what: &str, schema: &Value) -> Result<(), String> {
    if !(schema.is_object() || schema.is_boolean()) {
        return Err(format!("The {what} is not an object or a boolean"));
    }
    walk(schema, 0, MAX_SCHEMA_DEPTH, true).map_err(|problem| problem.text(what, MAX_SCHEMA_DEPTH))
}

/// What [`walk`] found.
enum Problem {
    Nul,
    TooDeep,
}

impl Problem {
    fn text(&self, what: &str, max: usize) -> String {
        match self {
            Problem::Nul => format!("The {what} contains a NUL character"),
            Problem::TooDeep => format!("The {what} is nested deeper than {max} levels"),
        }
    }
}

/// Walks `value`, which is inside `depth` arrays and objects, looking for a NUL character when `nul` is set. It calls
/// itself at most `max` times: a container inside `max` others is refused before its items are walked.
fn walk(value: &Value, depth: usize, max: usize, nul: bool) -> Result<(), Problem> {
    let nested = |depth: usize| if depth < max { Ok(depth + 1) } else { Err(Problem::TooDeep) };
    match value {
        Value::Object(o) => {
            let inner = nested(depth)?;
            for (k, v) in o {
                if nul && k.contains('\0') {
                    return Err(Problem::Nul);
                }
                walk(v, inner, max, nul)?;
            }
            Ok(())
        }
        Value::Array(a) => {
            let inner = nested(depth)?;
            a.iter().try_for_each(|v| walk(v, inner, max, nul))
        }
        Value::String(s) if nul && s.contains('\0') => Err(Problem::Nul),
        _ => Ok(()),
    }
}

fn field(what: &str, value: &str, max: usize) -> Result<(), String> {
    let length = value.chars().count();
    if length > max {
        return Err(format!("The {what} is too long: {length} characters, at most {max}"));
    }
    if value.contains('\0') {
        return Err(format!("The {what} contains a NUL character"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn checks_fields() {
        let id = MessageId::new("x").unwrap();
        assert!(check(&id, "Run", None, &BTreeMap::new()).is_ok());
        assert!(check(&id, &"t".repeat(201), None, &BTreeMap::new()).is_err());
        assert!(check(&id, "a\0b", None, &BTreeMap::new()).is_err());
        let many: BTreeMap<String, String> = (0..65).map(|i| (format!("h{i}"), "v".to_owned())).collect();
        assert!(check(&id, "Run", None, &many).unwrap_err().contains("At most 64"));
        assert!(check_json("payload", Some(&json!({"a": ["x\u{0}"]}))).is_err());
        assert!(check_json("payload", Some(&json!({"a\u{0}": 1}))).is_err());
        assert!(check_json("payload", Some(&json!({"a": 1}))).is_ok());
    }

    /// A value `levels` deep: `wrap` puts one more level around the innermost value `leaf`, without recursion.
    fn nested(levels: usize, leaf: Value, wrap: impl Fn(usize, Value) -> Value) -> Value {
        (0..levels).fold(leaf, |inner, level| wrap(level, inner))
    }

    fn in_array(_: usize, inner: Value) -> Value {
        Value::Array(vec![json!(1), inner])
    }

    fn in_object(_: usize, inner: Value) -> Value {
        json!({"a": 1, "next": inner})
    }

    fn mixed(level: usize, inner: Value) -> Value {
        if level.is_multiple_of(2) { in_array(level, inner) } else { in_object(level, inner) }
    }

    /// Scenario of the limit (value-depth): 64 levels are accepted and 65 refused, in arrays, in objects and in both.
    #[test]
    fn a_value_of_64_levels_is_accepted_and_one_of_65_refused() {
        type Wrap = fn(usize, Value) -> Value;
        for (shape, wrap) in [("arrays", in_array as Wrap), ("objects", in_object), ("mixed", mixed)] {
            // The innermost level is a container of its own: `[]`, then 63 more around it.
            let at_limit = nested(MAX_DEPTH - 1, json!([]), wrap);
            assert_eq!(check_json("payload", Some(&at_limit)), Ok(()), "{shape}");
            assert_eq!(check_depth("state", &at_limit), Ok(()), "{shape}");
            let beyond = wrap(0, at_limit);
            let refused = "The payload is nested deeper than 64 levels";
            assert_eq!(check_json("payload", Some(&beyond)).unwrap_err(), refused, "{shape}");
            assert_eq!(check_depth("state", &beyond).unwrap_err(), "The state is nested deeper than 64 levels");
        }
        assert_eq!(check_json("payload", Some(&nested(MAX_DEPTH, json!(1), in_object))), Ok(()), "a scalar is 0 deep");
        assert!(check_json("payload", Some(&nested(MAX_DEPTH, json!({}), in_object))).is_err());
    }

    #[test]
    fn a_scalar_is_0_deep_and_an_empty_container_1() {
        for scalar in [json!(null), json!(true), json!(1.5), json!("x")] {
            assert_eq!(check_json("state", Some(&scalar)), Ok(()));
        }
        assert_eq!(check_json("state", None), Ok(()));
        let one_deep = |v: Value| nested(MAX_DEPTH - 1, v, in_array);
        assert_eq!(check_json("state", Some(&one_deep(json!([])))), Ok(()));
        assert!(check_json("state", Some(&one_deep(json!([[]])))).is_err());
    }

    /// A NUL character at the limit is found, in a string and in a key; the depth check alone ignores it.
    #[test]
    fn a_nul_character_at_depth_is_found() {
        for leaf in [json!(["x\u{0}"]), json!({"a\u{0}": 1})] {
            let deep = nested(MAX_DEPTH - 1, leaf, mixed);
            assert_eq!(check_json("payload", Some(&deep)).unwrap_err(), "The payload contains a NUL character");
            assert_eq!(check_depth("payload", &deep), Ok(()));
        }
    }

    /// Scenario: A very deep value. 100,000 levels are refused on a thread with a stack of 256 KiB, naming the limit.
    /// The value is built without recursion and leaked: serde_json drops it with a walk that calls itself, which would
    /// overflow.
    #[test]
    fn a_value_100000_levels_deep_is_refused_on_a_small_stack() {
        let check = || {
            let value = nested(100_000, json!(1), |_, inner| Value::Array(vec![inner]));
            let refused = (check_json("payload", Some(&value)), check_depth("payload", &value));
            std::mem::forget(value);
            refused
        };
        let refused = std::thread::Builder::new().stack_size(256 << 10).spawn(check).unwrap().join().unwrap();
        let expected = Err("The payload is nested deeper than 64 levels".to_owned());
        assert_eq!(refused, (expected.clone(), expected));
    }
}
