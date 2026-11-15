//! The portable fixtures in `fixtures/` (ADR 0047, spec `conformance-fixtures`) against the implementation, through
//! public APIs only: canonical text and hashes, contract hashes, and the lease bodies. The expectations are reviewed
//! literals; no variable rewrites them, so a change of the implementation or a dependency that changes a covered
//! output fails here.

use std::collections::BTreeSet;
use std::path::PathBuf;

use cicada::contract::{Contract, canonical_json, canonical_sha256};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures"))
}

/// The fixture file `name`, parsed. Every JSON input in it is a string, so parsing the file loses no number.
fn fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(dir().join(name)).unwrap_or_else(|e| panic!("fixtures/{name}: {e}"));
    let value: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("fixtures/{name}: {e}"));
    assert_eq!(value["format"], 1, "fixtures/{name} has format 1");
    value
}

/// The cases of `list` in `file`, with unique names.
fn cases<'a>(file: &'a Value, list: &str) -> &'a [Value] {
    let cases = file[list].as_array().unwrap_or_else(|| panic!("no list {list}"));
    let mut names = BTreeSet::new();
    for case in cases {
        let name = case["name"].as_str().unwrap_or_else(|| panic!("a case of {list} without a name: {case}"));
        assert!(names.insert(name), "{list} names {name} twice");
    }
    cases
}

fn text<'a>(case: &'a Value, field: &str) -> &'a str {
    case[field].as_str().unwrap_or_else(|| panic!("{}: {field} is not a string", case["name"]))
}

fn parse(case: &Value, field: &str) -> Value {
    serde_json::from_str(text(case, field)).unwrap_or_else(|e| panic!("{}: {field}: {e}", case["name"]))
}

/// `sha256:` and the lowercase hex SHA-256 of the exact bytes of `text`.
fn sha256_of(text: &str) -> String {
    let hex: String = Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn names(cases: &[Value]) -> BTreeSet<&str> {
    cases.iter().map(|c| c["name"].as_str().unwrap()).collect()
}

/// type-registry, Scenarios: Integer and float representations; Float digits and layout; Keys and string escapes.
#[test]
fn every_valid_canonical_case_gives_its_exact_text_and_hash_and_is_idempotent() {
    let file = fixture("canonical.json");
    assert_eq!(file["baseline"]["serde_json"], "1.0.151");
    assert_eq!(file["baseline"]["zmij"], "1.0.23");
    let valid = cases(&file, "valid");
    for case in valid {
        let name = &case["name"];
        let value = parse(case, "input");
        let expected = text(case, "canonical");
        assert_eq!(canonical_json(&value), expected, "{name}: canonical text");
        assert_eq!(canonical_sha256(&value), text(case, "hash"), "{name}: hash");
        assert_eq!(sha256_of(expected), text(case, "hash"), "{name}: the hash is of the canonical UTF-8 bytes");
        assert_eq!(canonical_json(&parse(case, "canonical")), expected, "{name}: canonical text is canonical");
    }
    let present = names(valid);
    for required in [
        "integer-zero",
        "float-zero",
        "negative-integer-zero-is-float",
        "exponent-integer-is-float",
        "above-u64-is-float",
        "i64-min",
        "u64-max",
        "smallest-subnormal",
        "largest-finite",
        "underflow-to-zero",
        "fixed-at-exponent-minus-5",
        "exponent-below-minus-5",
        "fixed-at-exponent-15",
        "exponent-at-16",
        "parse-tie-to-even-down",
        "power-of-two-minus-1017",
        "keys-utf8-not-utf16-order",
        "keys-composed-and-decomposed-distinct",
        "string-escapes",
        "raw-controls-above-0x1f",
    ] {
        assert!(present.contains(required), "canonical.json lacks the case {required}");
    }
}

/// type-registry, Scenario: Integer and float representations (refused nonfinite overflow, malformed JSON).
#[test]
fn every_invalid_canonical_input_is_refused_by_the_parser() {
    let file = fixture("canonical.json");
    let invalid = cases(&file, "invalid");
    for case in invalid {
        let result = serde_json::from_str::<Value>(text(case, "input"));
        assert!(result.is_err(), "{}: parsed as {result:?}", case["name"]);
    }
    let present = names(invalid);
    for required in ["unpaired-high-surrogate", "unpaired-low-surrogate", "overflow-to-infinity", "nan"] {
        assert!(present.contains(required), "canonical.json lacks the invalid case {required}");
    }
    let valid = names(cases(&file, "valid"));
    assert!(valid.is_disjoint(&present), "a name is both valid and invalid");
}

/// type-registry, Scenario: Hash inputs and descriptions.
#[test]
fn every_contract_case_gives_its_hash_which_hashes_its_stripped_canonical_text() {
    let file = fixture("contracts.json");
    let contracts = cases(&file, "cases");
    for case in contracts {
        let name = &case["name"];
        let contract = Contract::from_json(&parse(case, "input")).unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = text(case, "canonical");
        assert_eq!(contract.hash(), text(case, "hash"), "{name}: contract hash");
        assert_eq!(sha256_of(expected), text(case, "hash"), "{name}: the hash is of the stripped canonical text");
        assert_eq!(canonical_json(&parse(case, "canonical")), expected, "{name}: stripped text is canonical");
    }
    let hash = |name: &str| {
        let case = contracts.iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("contracts.json: {name}"));
        text(case, "hash").to_owned()
    };
    // Contracts that differ only in schema descriptions share their hash.
    assert_eq!(hash("descriptions-removed"), hash("descriptions-absent"));
    assert_eq!(hash("descriptions-at-every-traversed-keyword"), hash("descriptions-at-every-traversed-keyword-absent"));
    assert_eq!(hash("minimal"), hash("unknown-top-level-keys-ignored"));
    // Data, names and unknown keywords called description stay in the hash input.
    let kept = contracts.iter().find(|c| c["name"] == "description-in-data-kept").unwrap();
    for data in ["const data", "enum data", "default data", "examples data", "an unknown keyword is not walked"] {
        assert!(text(kept, "canonical").contains(data), "{data} is kept");
    }
    assert!(text(kept, "canonical").contains(r#""messages":{"description":"#), "a message named description");
}
