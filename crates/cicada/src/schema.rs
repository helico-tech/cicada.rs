//! JSON Schemas (draft 2020-12) of states and messages, generated with schemars from the same types serde decodes.
//!
//! Schemas are published in the contract and served by `GET /types`; nothing validates against them. The decoder is
//! the only validator.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

use crate::message::{MessageError, TYPE_TAG};

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The schema of `T` as a standalone document: `$schema`, `title`, the body and the `$defs` it uses.
pub fn root_schema<T: JsonSchema>() -> Value {
    let generator = SchemaSettings::draft2020_12().into_generator();
    let schema = generator.into_root_schema_for::<T>();
    normalise(schema.to_value())
}

/// One schema per type name of the message family `M` (an enum with `#[serde(tag = "type")]`), each without the
/// tag property, as a standalone document titled with the type name.
pub fn message_schemas<M: JsonSchema>() -> Result<BTreeMap<String, Value>, MessageError> {
    let family = std::any::type_name::<M>();
    let not_tagged = || {
        MessageError::new(format!(
            "{family}: the message family must be an enum with #[serde(tag = \"type\")] whose variants are objects"
        ))
    };
    let generator = SchemaSettings::draft2020_12().into_generator();
    let root = generator.into_root_schema_for::<M>().to_value();
    let defs = root.get("$defs").and_then(Value::as_object).cloned().unwrap_or_default();
    let alternatives: Vec<Value> = match root.get("oneOf").and_then(Value::as_array) {
        Some(alts) => alts.clone(),
        // A family of one variant is a single object schema.
        None if root.get("properties").and_then(|p| p.get(TYPE_TAG)).is_some() => vec![root.clone()],
        None => return Err(not_tagged()),
    };
    let mut schemas = BTreeMap::new();
    for alternative in alternatives {
        let mut alt = alternative.as_object().cloned().ok_or_else(not_tagged)?;
        let name = alt
            .get("properties")
            .and_then(|p| p.get(TYPE_TAG))
            .and_then(|t| t.get("const").or_else(|| t.get("enum").and_then(|e| e.get(0))))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(not_tagged)?;
        // Remove the tag: the type name travels in the envelope.
        if let Some(Value::Object(props)) = alt.get_mut("properties") {
            props.remove(TYPE_TAG);
        }
        if let Some(Value::Array(required)) = alt.get_mut("required") {
            required.retain(|r| r != TYPE_TAG);
        }
        // A newtype variant refers to its struct: inline it.
        if let Some(Value::String(reference)) = alt.remove("$ref") {
            let target = reference
                .strip_prefix("#/$defs/")
                .and_then(|n| defs.get(n))
                .and_then(Value::as_object)
                .ok_or_else(not_tagged)?;
            let own_description = alt.remove("description");
            alt = target.clone();
            if let Some(d) = own_description {
                alt.entry("description").or_insert(d);
            }
        }
        if alt.get("properties").and_then(Value::as_object).is_some_and(Map::is_empty) {
            alt.insert("properties".into(), Value::Object(Map::new()));
        }
        if alt.get("required").and_then(Value::as_array).is_some_and(Vec::is_empty) {
            alt.remove("required");
        }
        alt.remove("$schema");
        alt.remove("title");
        let body = Value::Object(alt);
        let used = referenced_defs(&body, &defs);
        let mut document = Map::new();
        document.insert("$schema".into(), Value::String(DRAFT.into()));
        document.insert("title".into(), Value::String(name.clone()));
        if let Value::Object(fields) = body {
            document.extend(fields);
        }
        if !used.is_empty() {
            document.insert(
                "$defs".into(),
                Value::Object(used.into_iter().filter_map(|n| defs.get(&n).map(|d| (n, d.clone()))).collect()),
            );
        }
        if schemas.insert(name.clone(), normalise(Value::Object(document))).is_some() {
            return Err(MessageError::new(format!("{family}: message type '{name}' is declared twice")));
        }
    }
    Ok(schemas)
}

/// The names of the `$defs` that `value` refers to, directly or through other defs.
fn referenced_defs(value: &Value, defs: &Map<String, Value>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut queue = vec![value.clone()];
    while let Some(v) = queue.pop() {
        collect_refs(&v, &mut |name| {
            if found.insert(name.to_owned())
                && let Some(def) = defs.get(name)
            {
                queue.push(def.clone());
            }
        });
    }
    found
}

fn collect_refs(value: &Value, found: &mut impl FnMut(&str)) {
    match value {
        Value::Object(o) => {
            if let Some(name) = o.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/")) {
                found(name);
            }
            o.values().for_each(|v| collect_refs(v, found));
        }
        Value::Array(a) => a.iter().for_each(|v| collect_refs(v, found)),
        _ => {}
    }
}

/// Drops `null` members and puts `$schema` and `title` first.
fn normalise(value: Value) -> Value {
    match value {
        Value::Object(o) => {
            let mut out = Map::new();
            for key in ["$schema", "title"] {
                if let Some(v) = o.get(key) {
                    out.insert(key.into(), v.clone());
                }
            }
            for (k, v) in o {
                if v.is_null() || out.contains_key(&k) {
                    continue;
                }
                out.insert(k, normalise(v));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(normalise).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct Charge {
        amount: i64,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "lowercase")]
    enum Color {
        Red,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Msg {
        Run,
        Paint { color: Color },
        Charge(Charge),
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Single {
        Only { x: u8 },
    }

    #[test]
    fn splits_a_family() {
        let schemas = message_schemas::<Msg>().unwrap();
        assert_eq!(schemas["Run"], json!({"$schema": DRAFT, "title": "Run", "type": "object", "properties": {}}));
        let paint = &schemas["Paint"];
        assert_eq!(paint["required"], json!(["color"]));
        assert!(paint["$defs"]["Color"].is_object());
        assert!(paint["properties"].get("type").is_none());
        assert_eq!(schemas["Charge"]["properties"]["amount"]["type"], "integer");
        assert!(schemas["Charge"].get("$defs").is_none());
        assert_eq!(message_schemas::<Single>().unwrap().keys().collect::<Vec<_>>(), vec!["Only"]);
    }

    #[test]
    fn refuses_untagged_families() {
        assert!(message_schemas::<Charge>().is_err());
        assert!(message_schemas::<Color>().is_err());
    }
}
