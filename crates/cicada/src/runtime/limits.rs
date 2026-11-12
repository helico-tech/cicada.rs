//! Limits on the envelope fields of a send. Within them every backend stores a message without error; beyond them a
//! send is refused before it reaches the store, the same way on every backend: from outside an actor as a bad request
//! (`400`), inside a turn with a [`crate::turn::TurnError::Invalid`] at the call. No field, and no string or key of a
//! payload or state, may contain a NUL character (PostgreSQL text and JSONB cannot hold one).

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

/// Checks that no string and no object key in `value` contains a NUL character.
pub(crate) fn check_json(what: &str, value: Option<&Value>) -> Result<(), String> {
    match value {
        Some(v) if contains_nul(v) => Err(format!("The {what} contains a NUL character")),
        _ => Ok(()),
    }
}

fn contains_nul(value: &Value) -> bool {
    match value {
        Value::Object(o) => o.iter().any(|(k, v)| k.contains('\0') || contains_nul(v)),
        Value::Array(a) => a.iter().any(contains_nul),
        Value::String(s) => s.contains('\0'),
        _ => false,
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
}
