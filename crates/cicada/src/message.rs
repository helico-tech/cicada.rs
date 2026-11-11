//! Message families and actor state: how typed values become the JSON a store keeps.
//!
//! A message family is an enum tagged with `#[serde(tag = "type")]`:
//!
//! ```
//! use schemars::JsonSchema;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Serialize, Deserialize, JsonSchema)]
//! #[serde(tag = "type")]
//! enum SyncMsg {
//!     Run,
//!     FetchPage { cursor: Option<String> },
//! }
//!
//! let encoded = cicada::message::encode(&SyncMsg::FetchPage { cursor: None }).unwrap();
//! assert_eq!(encoded.message_type, "FetchPage");
//! assert_eq!(encoded.payload, serde_json::json!({"cursor": null}));
//! ```
//!
//! A message travels as its type name (the variant name, or its `#[serde(rename)]`) and its payload: the variant's
//! fields as a JSON object, without the tag. The type name lives in the envelope's `type` field, so a variant must not
//! have a field named `type`. Payloads and states are decoded leniently: unknown keys are ignored (unless a type opts
//! into `#[serde(deny_unknown_fields)]`), so additive changes stay decodable.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::schema;

/// The JSON property that carries a message's type name inside the family's own serde form.
pub const TYPE_TAG: &str = "type";

/// A message that cannot be encoded or decoded, or a type that is not a message family. It prints why, e.g.
/// "Payload of 'Add' cannot be decoded: invalid type: string \"x\", expected i64".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct MessageError(String);

impl MessageError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The root of an actor's messages: an enum with `#[serde(tag = "type")]` deriving `Serialize`, `Deserialize` and
/// `JsonSchema`. Implemented for every such type.
pub trait MessageFamily: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static {}

impl<T: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static> MessageFamily for T {}

/// A message as it travels: its type name and its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedMessage {
    /// The type name.
    pub message_type: String,
    /// The payload: the variant's fields, without the type tag.
    pub payload: Value,
}

/// Encodes a message of a family into its type name and payload.
pub fn encode<M: Serialize>(message: &M) -> Result<EncodedMessage, MessageError> {
    let value = serde_json::to_value(message).map_err(|e| MessageError(format!("Message cannot be encoded: {e}")))?;
    let not_a_family = || {
        MessageError(format!(
            "{} is not a message family: it must be an enum with #[serde(tag = \"type\")]",
            std::any::type_name::<M>()
        ))
    };
    let Value::Object(mut fields) = value else {
        return Err(not_a_family());
    };
    match fields.remove(TYPE_TAG) {
        Some(Value::String(message_type)) => Ok(EncodedMessage { message_type, payload: Value::Object(fields) }),
        _ => Err(not_a_family()),
    }
}

/// Decodes `payload` as the variant named `message_type`. The error message is the reason recorded for a message
/// that cannot be decoded.
pub fn decode<M: DeserializeOwned>(message_type: &str, payload: &Value) -> Result<M, MessageError> {
    let Value::Object(fields) = payload else {
        return Err(MessageError(format!("Payload of '{message_type}' must be a JSON object")));
    };
    let mut tagged = Map::with_capacity(fields.len() + 1);
    tagged.insert(TYPE_TAG.to_owned(), Value::String(message_type.to_owned()));
    for (k, v) in fields {
        if k != TYPE_TAG {
            tagged.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(Value::Object(tagged))
        .map_err(|e| MessageError(format!("Payload of '{message_type}' cannot be decoded: {e}")))
}

/// What the runtime knows of a message family: its type names and the JSON Schema of each.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyInfo {
    /// JSON Schema per type name, sorted by name.
    pub schemas: BTreeMap<String, Value>,
}

impl FamilyInfo {
    /// Analyses the family `M`; fails when it is not an enum tagged with `type`.
    pub fn of<M: MessageFamily>() -> Result<Self, MessageError> {
        Ok(Self { schemas: schema::message_schemas::<M>()? })
    }

    /// True when `message_type` is one of the family's type names.
    pub fn accepts(&self, message_type: &str) -> bool {
        self.schemas.contains_key(message_type)
    }
}

/// The state of an actor type that keeps no data: an empty struct, stored as `{}`. A state with a field does not
/// decode as `NoState`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoState {}

/// Refuses a state that is JSON `null`; `what` names it in the error ("initial state", "state").
pub(crate) fn check_not_null(what: &str, state: &Value) -> Result<(), String> {
    if state.is_null() {
        return Err(format!("The {what} is null; a state is never null (a type that keeps no data uses NoState)"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    struct Charge {
        amount: i64,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Msg {
        Run,
        FetchPage { cursor: Option<String> },
        Charge(Charge),
    }

    #[test]
    fn encodes_and_decodes() {
        let e = encode(&Msg::Run).unwrap();
        assert_eq!(e, EncodedMessage { message_type: "Run".into(), payload: json!({}) });
        let e = encode(&Msg::Charge(Charge { amount: 3 })).unwrap();
        assert_eq!(e.payload, json!({"amount": 3}));
        assert_eq!(
            decode::<Msg>("Charge", &json!({"amount": 3, "extra": true})).unwrap(),
            Msg::Charge(Charge { amount: 3 })
        );
        assert_eq!(decode::<Msg>("FetchPage", &json!({})).unwrap(), Msg::FetchPage { cursor: None });
        assert!(decode::<Msg>("Nope", &json!({})).is_err());
        assert!(decode::<Msg>("Run", &json!([])).is_err());
        assert!(encode(&5).is_err());
    }

    #[test]
    fn family_info() {
        let info = FamilyInfo::of::<Msg>().unwrap();
        assert_eq!(info.schemas.keys().collect::<Vec<_>>(), vec!["Charge", "FetchPage", "Run"]);
        assert!(info.accepts("Run"));
    }

    #[test]
    fn no_state_encodes_to_an_empty_object_and_refuses_fields() {
        assert_eq!(serde_json::to_value(NoState {}).unwrap(), json!({}));
        assert_eq!(serde_json::from_value::<NoState>(json!({})).unwrap(), NoState {});
        assert!(serde_json::from_value::<NoState>(json!({"oops": 1})).is_err());
        assert!(serde_json::from_value::<NoState>(Value::Null).is_err());
        assert_eq!(schema::root_schema::<NoState>()["type"], "object");
    }
}
