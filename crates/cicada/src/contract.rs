//! The published description of an actor type and its stable hash.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::address::{ActorAddress, TypeName};
use crate::store::TypeDef;

/// The published description of an actor type: its name and key fields, the JSON Schema of its state (`None` for a stateless
/// type) and one JSON Schema per message type name.
///
/// Its JSON form ([`Self::to_json`]) is what the TypeRegistry stores and `GET /types` serves:
/// `{"type": "sync", "key": ["tenant"], "state": {...} | null, "messages": {"FetchPage": {...}, "Run": {...}}}`.
/// [`Self::hash`] is the SHA-256 of the canonical JSON; two nodes registering the same name must produce the same
/// hash.
#[derive(Debug, Clone)]
pub struct Contract {
    type_name: TypeName,
    key_fields: Vec<String>,
    state_schema: Option<Value>,
    message_schemas: BTreeMap<String, Value>,
    hash: String,
}

impl Contract {
    /// A contract of `type_name`, whose key is made of `key_fields`, with the given schemas.
    pub fn new(
        type_name: TypeName,
        key_fields: Vec<String>,
        state_schema: Option<Value>,
        message_schemas: BTreeMap<String, Value>,
    ) -> Self {
        let mut contract = Self { type_name, key_fields, state_schema, message_schemas, hash: String::new() };
        contract.hash = canonical_sha256(&contract.to_json());
        contract
    }

    /// The type's name.
    pub fn type_name(&self) -> &TypeName {
        &self.type_name
    }

    /// The names of the ref's fields that form the key, in order; empty for a singleton.
    pub fn key_fields(&self) -> &[String] {
        &self.key_fields
    }

    /// The values of `address`'s key by field name; empty for a singleton.
    pub fn params_of(&self, address: &ActorAddress) -> BTreeMap<String, String> {
        self.key_fields.iter().cloned().zip(address.key_values()).collect()
    }

    /// The JSON Schema of the state, or `None` for a stateless type.
    pub fn state_schema(&self) -> Option<&Value> {
        self.state_schema.as_ref()
    }

    /// JSON Schema per message type name, sorted by name.
    pub fn message_schemas(&self) -> &BTreeMap<String, Value> {
        &self.message_schemas
    }

    /// True when the type accepts `message_type`.
    pub fn accepts(&self, message_type: &str) -> bool {
        self.message_schemas.contains_key(message_type)
    }

    /// The stable hash: `sha256:` + hex of the canonical JSON.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// The contract as JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "type": self.type_name.as_str(),
            "key": self.key_fields,
            "state": self.state_schema,
            "messages": Value::Object(self.message_schemas.clone().into_iter().collect::<Map<_, _>>()),
        })
    }

    /// The registry entry for this contract.
    pub fn to_type_def(&self) -> TypeDef {
        TypeDef { name: self.type_name.clone(), contract: self.to_json(), contract_hash: self.hash.clone() }
    }

    /// Parses a contract from its JSON form.
    pub fn from_json(json: &Value) -> Result<Self, String> {
        let o = json.as_object().ok_or("Not a contract: not an object")?;
        let type_name = o
            .get("type")
            .and_then(Value::as_str)
            .ok_or("Not a contract: no type")
            .and_then(|t| TypeName::parse(t).map_err(|_| "Not a contract: bad type name"))?;
        let key_fields = o
            .get("key")
            .and_then(Value::as_array)
            .ok_or("Not a contract: no key")?
            .iter()
            .map(|f| f.as_str().map(str::to_owned).ok_or("Not a contract: bad key"))
            .collect::<Result<Vec<_>, _>>()?;
        let state_schema = o.get("state").filter(|s| !s.is_null()).cloned();
        let messages = o
            .get("messages")
            .and_then(Value::as_object)
            .ok_or("Not a contract: no messages")?
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Ok(Self::new(type_name, key_fields, state_schema, messages))
    }

    /// Parses the contract of a registry entry, keeping its registered hash.
    pub fn of(def: &TypeDef) -> Result<Self, String> {
        let mut contract = Self::from_json(&def.contract)?;
        contract.hash = def.contract_hash.clone();
        Ok(contract)
    }
}

impl PartialEq for Contract {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

/// Canonical JSON text: object keys sorted (by their bytes), no insignificant whitespace, array order kept.
/// Two equal JSON values always yield the same text, so the text is a stable input for hashing.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(o) => {
            let mut entries: Vec<(&String, &Value)> = o.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (i, (k, v)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canonical(v, out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(v, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Lower-case hex SHA-256 of the canonical text of `value`, prefixed with `sha256:`.
pub fn canonical_sha256(value: &Value) -> String {
    let digest = Sha256::digest(canonical_json(value).as_bytes());
    let mut hex = String::with_capacity(7 + 64);
    hex.push_str("sha256:");
    for byte in digest.iter() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_text_sorts_keys() {
        let a = json!({"b": 1, "a": [3, {"d": null, "c": "x"}]});
        assert_eq!(canonical_json(&a), r#"{"a":[3,{"c":"x","d":null}],"b":1}"#);
        let b: Value = serde_json::from_str(r#"{"a":[3,{"c":"x","d":null}],"b":1}"#).unwrap();
        assert_eq!(canonical_sha256(&a), canonical_sha256(&b));
        assert!(canonical_sha256(&a).starts_with("sha256:"));
    }

    #[test]
    fn contract_round_trip() {
        let mut messages = BTreeMap::new();
        messages.insert("Run".to_owned(), json!({"type": "object"}));
        let c = Contract::new(TypeName::parse("sync").unwrap(), vec!["tenant".to_owned()], None, messages);
        let parsed = Contract::from_json(&c.to_json()).unwrap();
        assert_eq!(parsed.hash(), c.hash());
        assert!(parsed.accepts("Run"));
        assert_eq!(c.to_json()["state"], Value::Null);
    }
}
