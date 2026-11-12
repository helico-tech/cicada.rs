//! JSON Schemas (draft 2020-12) of states and messages, generated with schemars from the same types serde decodes.
//!
//! Schemas are published in the contract and served by `GET /types`. The core has no validator: the HTTP door of
//! `cicada-axum` checks a send and a create against them, through [`SchemaCheck`] (ADR 0026). At the turn the decoder
//! is the only judge.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

use crate::message::{MessageError, TYPE_TAG};

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Checks a payload or a state against its registered schema: `check(schema, value)`, true when it matches. It names no
/// place: finding one can cost without bound for a type with several shapes (ADR 0026, decision 8 of the change).
/// [`ActorSystem::send_message_checked`] and [`ActorSystem::create_actor_checked`] take one from their caller.
///
/// [`ActorSystem::send_message_checked`]: crate::ActorSystem::send_message_checked
/// [`ActorSystem::create_actor_checked`]: crate::ActorSystem::create_actor_checked
pub type SchemaCheck<'a> = &'a (dyn Fn(&Value, &Value) -> bool + Send + Sync);

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
        // A newtype variant refers to its struct: inline it. A pointer inside a definition would inline more than the
        // variant decodes, and the door may not be stricter than the decoder (ADR 0026).
        if let Some(Value::String(reference)) = alt.remove("$ref") {
            let (definition, inside) = definition_name(&reference).ok_or_else(not_tagged)?;
            if inside {
                return Err(MessageError::new(format!(
                    "{family}: the variant '{name}' refers to '{reference}', inside the definition '{definition}'; a \
                     newtype variant refers to a whole definition"
                )));
            }
            let target = defs
                .get(&definition)
                .ok_or_else(|| {
                    MessageError::new(format!(
                        "{family}: the variant '{name}' refers to the definition '{definition}', which the schema of \
                         the family does not hold"
                    ))
                })?
                .as_object()
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
        let mut document = Value::Object(document);
        accept_the_family(&mut document);
        if schemas.insert(name.clone(), normalise(document)).is_some() {
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
            if let Some((name, _)) = o.get("$ref").and_then(Value::as_str).and_then(definition_name) {
                found(&name);
            }
            o.values().for_each(|v| collect_refs(v, found));
        }
        Value::Array(a) => a.iter().for_each(|v| collect_refs(v, found)),
        _ => {}
    }
}

/// The name of the definition that `reference` points into, for a `$ref` that starts with `#/$defs/`, and whether it
/// points inside that definition: the first segment of the pointer, read as RFC 6901 reads a pointer in the fragment of
/// an address, and whether more segments follow. The rest is percent-decoded (left as it is when a `%` is not followed
/// by two hex digits or the bytes are not UTF-8), split at `/`, and `~1` and `~0` in the segment become `/` and `~`.
/// schemars writes the name `Line <v2>` as `#/$defs/Line%20%3Cv2%3E`, `line/v2` as `#/$defs/line~1v2`;
/// `#/$defs/Line/properties/sku` needs the definition `Line` and points inside it.
fn definition_name(reference: &str) -> Option<(String, bool)> {
    let rest = reference.strip_prefix("#/$defs/")?;
    let pointer = percent_decode(rest).unwrap_or_else(|| rest.to_owned());
    let (segment, inside) = pointer.split_once('/').map_or((pointer.as_str(), false), |(first, _)| (first, true));
    Some((segment.replace("~1", "/").replace("~0", "~"), inside))
}

/// `text` with each `%` and two hex digits turned into its byte; `None` when a `%` is not followed by two hex digits or
/// the bytes are not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let hex = |b: u8| char::from(b).to_digit(16);
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        rest = tail;
        if byte == b'%' {
            let (high, low) = (hex(*tail.first()?)?, hex(*tail.get(1)?)?);
            bytes.push((high * 16 + low) as u8);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

/// Makes every reference to the whole family (`"$ref": "#"`, which schemars writes for a family that holds itself) in
/// the message document `schema` accept any value: in the document `#` would be the one message. A subschema that is
/// only the reference becomes `true`; one with other keywords loses the reference. The decoder checks such a nested
/// message. Only keywords that hold schemas are walked: a `#` in `const`, `enum`, `default` or `examples` is data.
fn accept_the_family(schema: &mut Value) {
    let Value::Object(o) = schema else { return };
    if o.get("$ref").and_then(Value::as_str) == Some("#") {
        o.shift_remove("$ref");
        if o.is_empty() {
            *schema = Value::Bool(true);
            return;
        }
    }
    for_each_subschema(o, &mut accept_the_family);
}

/// Calls `f` on each subschema directly under the schema object `o`: the values of the keywords that hold schemas. The
/// keys of `properties` and the like are names, and `const`, `enum`, `default` and `examples` hold data, so they are
/// not walked.
pub(crate) fn for_each_subschema(o: &mut Map<String, Value>, f: &mut impl FnMut(&mut Value)) {
    for (keyword, value) in o.iter_mut() {
        match keyword.as_str() {
            "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
                if let Value::Object(named) = value {
                    named.values_mut().for_each(&mut *f);
                }
            }
            "items" | "prefixItems" | "anyOf" | "oneOf" | "allOf" => match value {
                Value::Array(list) => list.iter_mut().for_each(&mut *f),
                single => f(single),
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
            | "propertyNames" => f(value),
            _ => {}
        }
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

    /// Names that schemars escapes in a reference, written as it writes them, the definition each reference needs, and
    /// whether it points inside it.
    #[test]
    fn a_reference_is_read_as_a_pointer_in_a_fragment() {
        for (reference, name, inside) in [
            ("#/$defs/Charge", "Charge", false),
            ("#/$defs/Line%20%3Cv2%3E", "Line <v2>", false),
            ("#/$defs/line~1v2", "line/v2", false),
            ("#/$defs/a~0b", "a~b", false),
            ("#/$defs/~01", "~1", false),
            ("#/$defs/Gr%C3%B6%C3%9Fe", "Größe", false),
            ("#/$defs/100%25", "100%", false),
            // A `%` that does not decode, and bytes that are not UTF-8: the rest as it is, split and unescaped.
            ("#/$defs/100%", "100%", false),
            ("#/$defs/50%zz~1off", "50%zz/off", false),
            ("#/$defs/%C3", "%C3", false),
            // A pointer of several segments needs the definition of its first; a decoded `/` splits too.
            ("#/$defs/Line/properties/sku", "Line", true),
            ("#/$defs/Line%2Fsku", "Line", true),
            ("#/$defs/Line/", "Line", true),
        ] {
            assert_eq!(definition_name(reference), Some((name.to_owned(), inside)), "{reference}");
        }
        for reference in ["#", "#/definitions/Charge", "other.json#/$defs/Charge", "#/$defs"] {
            assert_eq!(definition_name(reference), None, "{reference}");
        }
    }

    /// A name that schemars escapes in several ways: a space, `<>`, `/`, `~`, `%`, `[]`, `|` and letters that are not
    /// ASCII.
    const ODD: &str = "a b<c>/d~e%f[g]|Größe";

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[schemars(rename = "a b<c>/d~e%f[g]|Größe")]
    struct Odd {
        x: u8,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Holds {
        Hold { odd: Odd },
    }

    #[test]
    fn the_reference_that_schemars_writes_for_an_escaped_name_needs_the_definition_of_that_name() {
        let root = SchemaSettings::draft2020_12().into_generator().into_root_schema_for::<Holds>().to_value();
        let reference = root["oneOf"][0]["properties"]["odd"]["$ref"].as_str().unwrap();
        assert_eq!(reference, "#/$defs/a%20b%3Cc%3E~1d~0e%25f%5Bg%5D%7CGr%C3%B6%C3%9Fe");
        assert_eq!(definition_name(reference), Some((ODD.to_owned(), false)));
        assert!(message_schemas::<Holds>().unwrap()["Hold"]["$defs"][ODD].is_object());
    }

    /// `Line <v2>`, through `#[schemars(rename)]`.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[schemars(rename = "Line <v2>")]
    struct Line {
        sku: String,
        color: Color,
    }

    /// `line/v2`, through a container `#[serde(rename)]`, which schemars follows.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename = "line/v2")]
    struct SerdeLine {
        sku: String,
    }

    /// A Rust name with letters that are not ASCII.
    #[derive(Serialize, Deserialize, JsonSchema)]
    struct Größe {
        cm: u32,
    }

    /// A type with a plain name that holds a type whose name a reference escapes.
    #[derive(Serialize, Deserialize, JsonSchema)]
    struct Holder {
        line: Line,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Order {
        Add { line: Line },
        AddV2 { line: SerdeLine },
        Measure { size: Größe },
        Hold { holder: Holder },
        Put(Line),
    }

    /// Scenarios: A renamed type; A type renamed for serde; A type name with a letter that is not ASCII. The message
    /// schema holds the definition, also one it reaches through another definition, and what the definition refers to.
    #[test]
    fn a_message_schema_holds_a_definition_whose_name_is_escaped_in_its_reference() {
        let schemas = message_schemas::<Order>().unwrap();
        for (message, name) in [("Add", "Line <v2>"), ("AddV2", "line/v2"), ("Measure", "Größe"), ("Hold", "Line <v2>")]
        {
            let defs = &schemas[message]["$defs"];
            assert_eq!(defs.as_object().map(|d| d.contains_key(name)), Some(true), "{message}: {defs}");
        }
        assert!(schemas["Add"]["$defs"]["Color"].is_object());
    }

    /// Scenario: A newtype variant of a renamed type. Its message schema is the schema of that type, with the
    /// definitions it refers to.
    #[test]
    fn a_newtype_variant_of_a_renamed_type_is_the_schema_of_that_type() {
        assert_eq!(
            message_schemas::<Order>().unwrap()["Put"],
            json!({"$schema": DRAFT, "title": "Put", "type": "object",
                "properties": {"sku": {"type": "string"}, "color": {"$ref": "#/$defs/Color"}},
                "required": ["sku", "color"],
                "description": "`Line <v2>`, through `#[schemars(rename)]`.",
                "$defs": {"Color": {"type": "string", "enum": ["red"]}}})
        );
    }

    /// A message family `$name` whose schema is written by hand.
    macro_rules! handwritten {
        ($name:ident, $schema:tt) => {
            struct $name;

            impl JsonSchema for $name {
                fn schema_name() -> std::borrow::Cow<'static, str> {
                    stringify!($name).into()
                }

                fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                    schemars::json_schema!($schema)
                }
            }
        };
    }

    handwritten!(Missing, {"oneOf": [
        {"type": "object", "properties": {"type": {"const": "Run"}}, "required": ["type"]},
        {"type": "object", "properties": {"type": {"const": "Line"}}, "required": ["type"], "$ref": "#/$defs/Gone%20v2"}
    ]});

    /// Scenario: A newtype variant whose definition is missing.
    #[test]
    fn a_newtype_variant_whose_definition_is_missing_names_the_variant_and_the_definition() {
        let e = message_schemas::<Missing>().unwrap_err().to_string();
        assert!(e.contains("the variant 'Line' refers to the definition 'Gone v2'"), "{e}");
        assert!(!e.contains("must be an enum"), "{e}");
    }

    handwritten!(Inside, {"oneOf": [
        {"type": "object", "properties": {"type": {"const": "Sku"}}, "required": ["type"],
            "$ref": "#/$defs/Line/properties/sku"}
    ], "$defs": {"Line": {"type": "object", "properties": {"sku": {"type": "object"}}, "required": ["sku"]}}});

    /// Scenario: A newtype variant that points inside a definition. Inlining the whole definition would make the door
    /// stricter than the decoder (ADR 0026), so the family is refused, naming the variant and the reference.
    #[test]
    fn a_newtype_variant_that_points_inside_a_definition_names_the_variant_and_the_reference() {
        let e = message_schemas::<Inside>().unwrap_err().to_string();
        assert!(
            e.contains("the variant 'Sku' refers to '#/$defs/Line/properties/sku', inside the definition 'Line'"),
            "{e}"
        );
        assert!(!e.contains("must be an enum"), "{e}");
    }

    handwritten!(Segments, {"oneOf": [
        {"type": "object", "properties": {"type": {"const": "Add"}, "sku": {"$ref": "#/$defs/Line/properties/sku"}},
            "required": ["type"]},
        {"type": "object", "properties": {"type": {"const": "Run"}}, "required": ["type"]}
    ], "$defs": {"Line": {"type": "object", "properties": {"sku": {"type": "string"}}}}});

    /// Scenario: A reference of several segments.
    #[test]
    fn a_reference_of_several_segments_needs_the_definition_of_its_first() {
        let schemas = message_schemas::<Segments>().unwrap();
        assert_eq!(
            schemas["Add"]["$defs"],
            json!({"Line": {"type": "object", "properties": {"sku": {"type": "string"}}}})
        );
        assert!(schemas["Run"].get("$defs").is_none());
    }

    /// A family that holds itself: in a field, an optional field with a doc comment, and a definition.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Tree {
        Add {
            n: i32,
        },
        Batch {
            items: Vec<Tree>,
            /// The one after.
            next: Option<Box<Tree>>,
            group: Group,
        },
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct Group {
        members: Vec<Tree>,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum Flat {
        Add { n: i32 },
    }

    /// Scenario: The other messages of the family. Only the document that refers to the family changes: there each
    /// reference to it accepts any value, also in a definition the document holds.
    #[test]
    fn a_reference_to_the_family_accepts_any_value_and_the_other_messages_do_not_change() {
        let schemas = message_schemas::<Tree>().unwrap();
        let add = serde_json::to_string(&schemas["Add"]).unwrap();
        assert_eq!(add, serde_json::to_string(&message_schemas::<Flat>().unwrap()["Add"]).unwrap());
        assert_eq!(
            add,
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Add","type":"object","properties":{"n":{"type":"integer","format":"int32"}},"required":["n"]}"#
        );
        let batch = &schemas["Batch"];
        assert_eq!(batch["properties"]["items"], json!({"type": "array", "items": true}));
        assert_eq!(
            batch["properties"]["next"],
            json!({"anyOf": [true, {"type": "null"}], "description": "The one after."})
        );
        assert_eq!(batch["$defs"]["Group"]["properties"]["members"], json!({"type": "array", "items": true}));
        assert!(!serde_json::to_string(batch).unwrap().contains(r##""$ref":"#""##), "{batch}");
    }

    handwritten!(Data, {"oneOf": [{"type": "object", "properties": {
        "type": {"const": "Mark"},
        "mark": {"const": "#"},
        "data": {"const": {"$ref": "#"}, "enum": [{"$ref": "#"}], "default": {"$ref": "#"}, "examples": [{"$ref": "#"}]},
        "next": {"$ref": "#"}
    }, "required": ["type"]}]});

    /// Scenario: A `#` that is data. It stays in `const`, `enum`, `default` and `examples`; a reference beside it does
    /// not.
    #[test]
    fn a_reference_to_the_family_in_data_stays() {
        assert_eq!(
            message_schemas::<Data>().unwrap()["Mark"]["properties"],
            json!({
                "mark": {"const": "#"},
                "data": {"const": {"$ref": "#"}, "enum": [{"$ref": "#"}], "default": {"$ref": "#"}, "examples": [{"$ref": "#"}]},
                "next": true
            })
        );
    }
}
