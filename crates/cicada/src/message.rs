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
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::schema;

/// The JSON property that carries a message's type name inside the family's own serde form.
pub const TYPE_TAG: &str = "type";

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
pub fn encode<M: Serialize>(message: &M) -> Result<EncodedMessage, String> {
    let value = serde_json::to_value(message).map_err(|e| format!("Message cannot be encoded: {e}"))?;
    let Value::Object(mut fields) = value else {
        return Err(format!(
            "{} is not a message family: it must be an enum with #[serde(tag = \"type\")]",
            std::any::type_name::<M>()
        ));
    };
    match fields.remove(TYPE_TAG) {
        Some(Value::String(message_type)) => Ok(EncodedMessage { message_type, payload: Value::Object(fields) }),
        _ => Err(format!(
            "{} is not a message family: it must be an enum with #[serde(tag = \"type\")]",
            std::any::type_name::<M>()
        )),
    }
}

/// Decodes `payload` as the variant named `message_type`. The error message is the reason recorded for an unhandled
/// message.
pub fn decode<M: DeserializeOwned>(message_type: &str, payload: &Value) -> Result<M, String> {
    let Value::Object(fields) = payload else {
        return Err(format!("Payload of '{message_type}' must be a JSON object"));
    };
    let mut tagged = Map::with_capacity(fields.len() + 1);
    tagged.insert(TYPE_TAG.to_owned(), Value::String(message_type.to_owned()));
    for (k, v) in fields {
        if k != TYPE_TAG {
            tagged.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(Value::Object(tagged))
        .map_err(|e| format!("Payload of '{message_type}' cannot be decoded: {e}"))
}

/// What the runtime knows of a message family: its type names and the JSON Schema of each.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyInfo {
    /// JSON Schema per type name, sorted by name.
    pub schemas: BTreeMap<String, Value>,
}

impl FamilyInfo {
    /// Analyses the family `M`; fails when it is not an enum tagged with `type`.
    pub fn of<M: MessageFamily>() -> Result<Self, String> {
        Ok(Self { schemas: schema::message_schemas::<M>()? })
    }

    /// True when `message_type` is one of the family's type names.
    pub fn accepts(&self, message_type: &str) -> bool {
        self.schemas.contains_key(message_type)
    }
}

/// The state of an actor type: any `Serialize + Deserialize + JsonSchema` type, or [`Stateless`].
pub trait ActorState: Send + Sync + Sized + 'static {
    /// True for [`Stateless`]: no state in the store and no state schema in the contract.
    const STATELESS: bool;

    /// Decodes the stored state (`None` for no state).
    fn decode(json: Option<&Value>) -> Result<Self, String>;

    /// Encodes the state for the store.
    fn encode(&self) -> Result<Option<Value>, String>;

    /// The JSON Schema of the state, or `None` for a stateless type.
    fn schema() -> Result<Option<Value>, String>;
}

impl<T: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static> ActorState for T {
    const STATELESS: bool = false;

    fn decode(json: Option<&Value>) -> Result<Self, String> {
        serde_json::from_value(json.cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())
    }

    fn encode(&self) -> Result<Option<Value>, String> {
        serde_json::to_value(self).map(Some).map_err(|e| e.to_string())
    }

    fn schema() -> Result<Option<Value>, String> {
        Ok(Some(schema::root_schema::<T>()))
    }
}

/// The state of a stateless actor type: its instances hold no state (null in the store), and its contract has no
/// state schema.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stateless;

impl ActorState for Stateless {
    const STATELESS: bool = true;

    fn decode(_: Option<&Value>) -> Result<Self, String> {
        Ok(Stateless)
    }

    fn encode(&self) -> Result<Option<Value>, String> {
        Ok(None)
    }

    fn schema() -> Result<Option<Value>, String> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
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
    fn stateless_has_no_state() {
        assert!(Stateless::schema().unwrap().is_none());
        assert_eq!(Stateless.encode().unwrap(), None);
        assert!(<Charge as ActorState>::schema().unwrap().is_some());
        assert!(<Charge as ActorState>::decode(Some(&json!({"amount": 1}))).is_ok());
        assert!(<Charge as ActorState>::decode(None).is_err());
    }
}
