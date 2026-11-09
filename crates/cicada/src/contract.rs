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
/// It keeps the schemas' `description`s (doc comments). [`Self::hash`] is the SHA-256 of the canonical JSON without
/// them (ADR 0017), so a doc comment change keeps the hash; two nodes registering the same name must produce the same
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
        contract.hash = canonical_sha256(&contract.hash_input());
        contract
    }

    /// [`Self::to_json`] with every `description` keyword removed from the state and message schemas.
    fn hash_input(&self) -> Value {
        let mut json = self.to_json();
        if let Some(state) = json.get_mut("state") {
            strip_descriptions(state);
        }
        if let Some(Value::Object(messages)) = json.get_mut("messages") {
            messages.values_mut().for_each(strip_descriptions);
        }
        json
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

    /// The stable hash: `sha256:` + hex of the canonical JSON, with the schemas' descriptions left out.
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

/// Removes the keyword `description` from `schema` and from every subschema in it. Only keywords that hold schemas are
/// walked: the keys of `properties` and the like are names (a property named `description` stays), and `const`, `enum`,
/// `default` and `examples` hold data.
fn strip_descriptions(schema: &mut Value) {
    let Value::Object(o) = schema else { return }; // a boolean schema has nothing to strip
    o.remove("description");
    for (keyword, value) in o.iter_mut() {
        match keyword.as_str() {
            "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
                if let Value::Object(named) = value {
                    named.values_mut().for_each(strip_descriptions);
                }
            }
            "items" | "prefixItems" | "anyOf" | "oneOf" | "allOf" => match value {
                Value::Array(list) => list.iter_mut().for_each(strip_descriptions),
                single => strip_descriptions(single),
            },
            "additionalProperties"
            | "additionalItems"
            | "unevaluatedProperties"
            | "unevaluatedItems"
            | "not"
            | "if"
            | "then"
            | "else"
            | "contains"
            | "propertyNames" => strip_descriptions(value),
            _ => {}
        }
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

    fn contract(state: Value, message: Value) -> Contract {
        let messages = BTreeMap::from([("Run".to_owned(), message)]);
        Contract::new(TypeName::parse("sync").unwrap(), vec!["tenant".to_owned()], Some(state), messages)
    }

    /// A state schema with a description at every place where schemars nests schemas, all set to `text`.
    fn documented(text: &str) -> Value {
        json!({
            "title": "State",
            "description": text,
            "type": "object",
            "properties": {
                "note": {"description": text, "type": "string"},
                "inner": {"type": "object", "properties": {"deep": {"description": text, "type": "integer"}}},
                "list": {"type": "array", "items": {"description": text, "type": "string"}},
                "either": {"anyOf": [{"description": text, "type": "string"}, {"type": "null"}]},
                "tuple": {"type": "array", "prefixItems": [{"description": text}]},
                "map": {"type": "object", "additionalProperties": {"description": text, "type": "string"}},
                "pick": {"oneOf": [{"description": text, "const": "a"}], "allOf": [{"description": text}]},
                "neg": {"not": {"description": text}},
                "cond": {"if": {"description": text}, "then": {"description": text}, "else": {"description": text}},
                "shared": {"$ref": "#/$defs/Shared"}
            },
            "patternProperties": {"^x-": {"description": text}},
            "$defs": {"Shared": {"description": text, "type": "object"}}
        })
    }

    #[test]
    fn descriptions_do_not_count_in_the_hash_but_stay_in_the_json() {
        let message = |text: &str| json!({"title": "Run", "description": text, "type": "object", "properties": {}});
        let old = contract(documented("old"), message("old"));
        let new = contract(documented("new, reworded"), message("new"));
        assert_eq!(old.hash(), new.hash());
        assert_eq!(old.to_json()["state"]["properties"]["inner"]["properties"]["deep"]["description"], "old");
        assert_eq!(new.to_json()["messages"]["Run"]["description"], "new");
        let mut bare = documented("x");
        strip_descriptions(&mut bare);
        assert!(!canonical_json(&bare).contains("description"), "{bare}");
        let without = contract(bare, json!({"title": "Run", "type": "object", "properties": {}}));
        assert_eq!(without.hash(), old.hash());
    }

    #[test]
    fn a_property_definition_or_message_named_description_still_counts() {
        let base = contract(json!({"type": "object", "properties": {}}), json!({"type": "object"}));
        let property = contract(
            json!({"type": "object", "properties": {"description": {"type": "string"}}}),
            json!({"type": "object"}),
        );
        let property_typed = contract(
            json!({"type": "object", "properties": {"description": {"type": "integer"}}}),
            json!({"type": "object"}),
        );
        let definition = contract(
            json!({"type": "object", "properties": {}, "$defs": {"description": {"type": "string"}}}),
            json!({"type": "object"}),
        );
        let in_default = contract(
            json!({"type": "object", "properties": {}, "default": {"description": "x"}}),
            json!({"type": "object"}),
        );
        let mut hashes: Vec<&str> =
            [&base, &property, &property_typed, &definition, &in_default].iter().map(|c| c.hash()).collect();
        hashes.sort();
        hashes.dedup();
        assert_eq!(hashes.len(), 5, "each contract has its own hash");

        let named = |schema: Value| {
            let messages = BTreeMap::from([("description".to_owned(), schema)]);
            Contract::new(TypeName::parse("sync").unwrap(), vec![], None, messages)
        };
        assert_ne!(named(json!({"type": "object"})).hash(), named(json!({"type": "string"})).hash());
        assert!(named(json!({"type": "object"})).accepts("description"));
    }
}
