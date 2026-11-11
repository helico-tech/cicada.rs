//! Contract snapshots and state samples: catch contract and state changes in a test run instead of at deploy time.

use std::fs;
use std::path::Path;

use cicada::contract::Contract;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// The environment variable that makes [`assert_contracts_match`] rewrite the snapshots instead of comparing.
pub const UPDATE_CONTRACTS_ENV: &str = "CICADA_UPDATE_CONTRACTS";

/// Asserts that `contracts` equal the snapshots in `directory`, so that a contract change shows up in a test run instead
/// of failing node start against the registry during a deploy (a node refuses a contract that differs from the one
/// registered for its type name). Rewrites differing snapshots when `CICADA_UPDATE_CONTRACTS=true`.
///
/// Each type has one file, `<directory>/<type name>.json`, holding its contract as `GET /types` serves it
/// (pretty printed, keys sorted). A missing file is written and the type passes, so the first run records the
/// snapshots; commit them. A file that differs fails the assertion with what changed: the key fields, the state schema,
/// and which message types were added, removed or changed. Formatting does not matter: contracts are compared by their
/// hash. Get a type's contract with [`cicada::actor::contract_of`].
///
/// ```no_run
/// # use cicada::actor::contract_of;
/// # fn types() -> Vec<cicada::contract::Contract> { vec![] }
/// cicada_test::assert_contracts_match("tests/contracts", &types());
/// ```
///
/// # Panics
///
/// Listing every type whose contract differs, or when two contracts map to the same file.
pub fn assert_contracts_match(directory: impl AsRef<Path>, contracts: &[Contract]) {
    let update = std::env::var(UPDATE_CONTRACTS_ENV).is_ok_and(|v| v == "true");
    if let Err(e) = check_contracts(directory.as_ref(), contracts, update) {
        panic!("{e}");
    }
}

/// [`assert_contracts_match`] as a result, with an explicit `update` flag.
pub fn check_contracts(directory: &Path, contracts: &[Contract], update: bool) -> Result<(), String> {
    let mut names: Vec<(String, &str)> =
        contracts.iter().map(|c| (snapshot_file_name(c), c.type_name().as_str())).collect();
    names.sort();
    for pair in names.windows(2) {
        if pair[0].0 == pair[1].0 {
            panic!("Types [{}, {}] share the snapshot file {}", pair[0].1, pair[1].1, pair[0].0);
        }
    }
    fs::create_dir_all(directory).map_err(|e| format!("Cannot create {}: {e}", directory.display()))?;
    let mut problems = Vec::new();
    for contract in contracts {
        let file = directory.join(snapshot_file_name(contract));
        let recorded = if file.exists() { Some(read_snapshot(&file)?) } else { None };
        match recorded {
            Some(r) if r.hash() == contract.hash() => {}
            Some(r) if !update => problems.push(format!(
                "{} ({}):\n{}",
                contract.type_name(),
                file.display(),
                describe_changes(&r, contract).iter().map(|c| format!("  - {c}")).collect::<Vec<_>>().join("\n")
            )),
            _ => {
                let text = serde_json::to_string_pretty(&sorted(&contract.to_json())).expect("JSON") + "\n";
                fs::write(&file, text).map_err(|e| format!("Cannot write {}: {e}", file.display()))?;
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Contracts differ from their snapshots:\n{}\nIf the change is intended, delete the snapshot or run with \
             {UPDATE_CONTRACTS_ENV}=true.",
            problems.join("\n")
        ))
    }
}

/// What changed from `old` to `new`, one line per change.
pub fn describe_changes(old: &Contract, new: &Contract) -> Vec<String> {
    let mut changes = Vec::new();
    if old.type_name() != new.type_name() {
        changes.push(format!("type name changed: {} -> {}", old.type_name(), new.type_name()));
    }
    if old.key_fields() != new.key_fields() {
        changes.push(format!(
            "key fields changed: [{}] -> [{}]",
            old.key_fields().join(", "),
            new.key_fields().join(", ")
        ));
    }
    if old.state_schema() != new.state_schema() {
        changes.push("state schema changed".to_owned());
    }
    let (old_m, new_m) = (old.message_schemas(), new.message_schemas());
    changes.extend(new_m.keys().filter(|k| !old_m.contains_key(*k)).map(|k| format!("message type added: {k}")));
    changes.extend(old_m.keys().filter(|k| !new_m.contains_key(*k)).map(|k| format!("message type removed: {k}")));
    changes.extend(
        old_m
            .iter()
            .filter(|(k, v)| new_m.get(*k).is_some_and(|n| n != *v))
            .map(|(k, _)| format!("message type changed: {k}")),
    );
    if changes.is_empty() {
        changes.push(format!("contract hash changed: {} -> {}", old.hash(), new.hash()));
    }
    changes
}

/// `sync` → `sync.json`: the type name, which is safe as a file name.
pub fn snapshot_file_name(contract: &Contract) -> String {
    format!("{}.json", contract.type_name())
}

fn read_snapshot(file: &Path) -> Result<Contract, String> {
    let text = fs::read_to_string(file).map_err(|e| format!("Snapshot {} cannot be read: {e}", file.display()))?;
    let json: Value =
        serde_json::from_str(&text).map_err(|e| format!("Snapshot {} is not a contract: {e}", file.display()))?;
    Contract::from_json(&json).map_err(|e| format!("Snapshot {} is not a contract: {e}", file.display()))
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&o[k]))).collect::<Map<_, _>>())
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// Writes `state` as a state sample to `<directory>/<name>.json`, unless that file exists: samples are a record of
/// states that instances in a store may hold, so an existing one is never overwritten. Use it with
/// [`assert_state_decodes`] to keep one sample per shape the state ever had. Returns true when the file was written.
///
/// # Panics
///
/// When `name` is not a plain file name or the file cannot be written.
pub fn record_state_sample<S: Serialize>(directory: impl AsRef<Path>, name: &str, state: &S) -> bool {
    let directory = directory.as_ref();
    assert!(
        !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || "._-".contains(c)),
        "Sample name must be letters, digits, '.', '_' or '-', was '{name}'"
    );
    let file = directory.join(format!("{name}.json"));
    if file.exists() {
        return false;
    }
    fs::create_dir_all(directory).expect("create the samples directory");
    let json = serde_json::to_string_pretty(&serde_json::to_value(state).expect("state encodes")).expect("JSON") + "\n";
    fs::write(&file, json).expect("write the sample");
    true
}

/// Asserts that every `*.json` file in `directory` decodes as state type `S`, the way a node decodes stored state. State
/// that does not decode fails every turn of its instance, so a change to a state type must keep old states readable:
/// add fields with `#[serde(default)]`, do not rename or remove required ones, do not change their types.
///
/// # Panics
///
/// Naming every sample that does not decode, or when `directory` holds no sample.
pub fn assert_state_decodes<S: DeserializeOwned>(directory: impl AsRef<Path>) {
    if let Err(e) = check_state_decodes::<S>(directory.as_ref()) {
        panic!("{e}");
    }
}

/// [`assert_state_decodes`] as a result.
pub fn check_state_decodes<S: DeserializeOwned>(directory: &Path) -> Result<(), String> {
    let type_name = std::any::type_name::<S>();
    let mut samples: Vec<_> = fs::read_dir(directory)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    samples.sort();
    if samples.is_empty() {
        return Err(format!("No state samples (*.json) in {} for {type_name}", directory.display()));
    }
    let problems: Vec<String> = samples
        .iter()
        .filter_map(|file| {
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let text = fs::read_to_string(file).ok()?;
            match serde_json::from_str::<S>(&text) {
                Ok(_) => None,
                Err(e) => Some(format!("{name}: {e}")),
            }
        })
        .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "State samples of {type_name} no longer decode; instances holding such state would fail every turn:\n{}",
            problems.iter().map(|p| format!("  - {p}")).collect::<Vec<_>>().join("\n")
        ))
    }
}
