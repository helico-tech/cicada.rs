//! The portable fixtures in `fixtures/` (ADR 0047, spec `conformance-fixtures`) against the implementation, through
//! public APIs only: canonical text and hashes, contract hashes, and the lease bodies. The expectations are reviewed
//! literals; no variable rewrites them, so a change of the implementation or a dependency that changes a covered
//! output fails here.

use std::collections::BTreeSet;
use std::path::PathBuf;

use cicada::WatchTarget;
use cicada::api::leases::{
    CompleteTurnRequest, CompleteTurnResponse, ControlEffectView, LeaseTurnsRequest, LeaseTurnsResponse,
    ReleaseTurnRequest, RenewTurnRequest, RenewTurnResponse, StateReportView, TurnMessageView, TurnOutcomeView,
    TurnReportView,
};
use cicada::contract::{Contract, canonical_json, canonical_sha256};
use cicada::retention::{Keep, Retention};
use cicada::retry::{Backoff, RetryPolicy};
use cicada::runtime::{CompleteTurnResult, LeaseTurnsResult, RenewTurnResult, TurnReport};
use cicada::watch::FailureCause;
use serde::Serialize;
use serde::de::DeserializeOwned;
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

// ---- lease bodies ----

/// Where a lease body is refused: by the typed decoder, or by the conversion to the core's types after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Decode,
    Conversion,
}

impl Stage {
    fn parse(text: &str) -> Stage {
        match text {
            "decode" => Stage::Decode,
            "conversion" => Stage::Conversion,
            other => panic!("unknown stage {other}"),
        }
    }
}

/// Every variant the valid lease cases must show, one label per arm of the matches below.
const VARIANTS: &[&str] = &[
    "LeaseTurnsResponse::Leased",
    "LeaseTurnsResponse::ContractMismatch",
    "LeaseTurnsResponse::ContractMismatch(registered null)",
    "RenewTurnResponse::Renewed",
    "RenewTurnResponse::LeaseLost",
    "CompleteTurnResponse::Completed",
    "CompleteTurnResponse::Rejected",
    "CompleteTurnResponse::LeaseLost",
    "TurnOutcomeView::Done",
    "TurnOutcomeView::Cancelled",
    "TurnOutcomeView::Failed",
    "TurnOutcomeView::Rescheduled(now)",
    "TurnOutcomeView::Rescheduled(deliverAt)",
    "TurnOutcomeView::Rescheduled(delay)",
    "TurnReportView::Returned",
    "TurnReportView::Failed",
    "TurnReportView::Cancelled",
    "StateReportView::Unchanged",
    "StateReportView::Set",
    "StateReportView::Set(null)",
    "StateReportView::Stop",
    "ControlEffectView::Pause",
    "ControlEffectView::Resume",
    "ControlEffectView::Purge",
    "ControlEffectView::Delete",
    "WatchTarget::Actor",
    "WatchTarget::Types",
    "WatchTarget::Every",
    "TurnSendView(now)",
    "TurnSendView(deliverAt)",
    "TurnSendView(delay)",
    "TurnEffectsView.creates(child)",
    "TurnEffectsView.creates(independent)",
    "TurnEffectsView.cancels",
    "FailureCause::Handler",
    "FailureCause::Node",
    "FailureCause::Refused",
    "FailureCause::Timeout",
    "FailureCause::Message",
    "FailureCause::State",
    "FailureCause::Commit",
    "FailureCause::ClaimLimit",
    "Backoff::Exponential",
    "Backoff::Fixed",
    "Keep::Remove",
    "Keep::For",
    "Keep::Last",
    "TurnMessageView(every nullable null)",
    "TurnMessageView(every nullable set)",
];

/// The labels of the variants in a lease body, by exhaustive matches: a variant added to a wire type does not compile
/// here until it has a label in [`VARIANTS`] and a case in `fixtures/leases.json`.
#[derive(Default)]
struct Seen(BTreeSet<&'static str>);

impl Seen {
    fn add(&mut self, label: &'static str) {
        self.0.insert(label);
    }

    fn cause(&mut self, cause: FailureCause) {
        self.add(match cause {
            FailureCause::Handler => "FailureCause::Handler",
            FailureCause::Node => "FailureCause::Node",
            FailureCause::Refused => "FailureCause::Refused",
            FailureCause::Timeout => "FailureCause::Timeout",
            FailureCause::Message => "FailureCause::Message",
            FailureCause::State => "FailureCause::State",
            FailureCause::Commit => "FailureCause::Commit",
            FailureCause::ClaimLimit => "FailureCause::ClaimLimit",
        });
    }

    fn retry(&mut self, retry: &RetryPolicy) {
        self.add(match retry.backoff() {
            Backoff::Exponential { .. } => "Backoff::Exponential",
            Backoff::Fixed { .. } => "Backoff::Fixed",
        });
    }

    fn retention(&mut self, retention: &Retention) {
        for keep in [&retention.done, &retention.failed].into_iter().flatten() {
            self.add(match keep {
                Keep::Remove => "Keep::Remove",
                Keep::For { .. } => "Keep::For",
                Keep::Last { .. } => "Keep::Last",
            });
        }
    }

    fn message(&mut self, m: &TurnMessageView) {
        let nullable = [
            m.from.is_some(),
            m.correlation_id.is_some(),
            m.last_error.is_some(),
            m.retry.is_some(),
            m.caused_by.is_some(),
            m.retention.is_some(),
        ];
        if nullable.iter().all(|set| !set) {
            self.add("TurnMessageView(every nullable null)");
        }
        if nullable.iter().all(|set| *set) {
            self.add("TurnMessageView(every nullable set)");
        }
        m.retry.iter().for_each(|r| self.retry(r));
        m.retention.iter().for_each(|r| self.retention(r));
    }

    fn lease(&mut self, r: &LeaseTurnsResponse) {
        match r {
            LeaseTurnsResponse::Leased { leases } => {
                self.add("LeaseTurnsResponse::Leased");
                leases.iter().for_each(|l| self.message(&l.message));
            }
            LeaseTurnsResponse::ContractMismatch { registered, .. } => {
                self.add("LeaseTurnsResponse::ContractMismatch");
                if registered.is_none() {
                    self.add("LeaseTurnsResponse::ContractMismatch(registered null)");
                }
            }
        }
    }

    fn renew(&mut self, r: &RenewTurnResponse) {
        self.add(match r {
            RenewTurnResponse::Renewed { .. } => "RenewTurnResponse::Renewed",
            RenewTurnResponse::LeaseLost => "RenewTurnResponse::LeaseLost",
        });
    }

    fn outcome(&mut self, o: &TurnOutcomeView) {
        self.add(match o {
            TurnOutcomeView::Done => "TurnOutcomeView::Done",
            TurnOutcomeView::Cancelled => "TurnOutcomeView::Cancelled",
            TurnOutcomeView::Failed { .. } => "TurnOutcomeView::Failed",
            TurnOutcomeView::Rescheduled { deliver_at, delay, .. } => match (deliver_at, delay) {
                (None, None) => "TurnOutcomeView::Rescheduled(now)",
                (Some(_), None) => "TurnOutcomeView::Rescheduled(deliverAt)",
                (None, Some(_)) => "TurnOutcomeView::Rescheduled(delay)",
                (Some(_), Some(_)) => "TurnOutcomeView::Rescheduled(both)",
            },
        });
    }

    fn complete(&mut self, r: &CompleteTurnResponse) {
        match r {
            CompleteTurnResponse::Completed { outcome, cause, sent, .. } => {
                self.add("CompleteTurnResponse::Completed");
                self.outcome(outcome);
                cause.iter().for_each(|c| self.cause(*c));
                sent.iter().for_each(|m| self.message(m));
            }
            CompleteTurnResponse::Rejected { failure } => {
                self.add("CompleteTurnResponse::Rejected");
                self.cause(failure.cause);
            }
            CompleteTurnResponse::LeaseLost => self.add("CompleteTurnResponse::LeaseLost"),
        }
    }

    fn report(&mut self, r: &TurnReportView) {
        match r {
            TurnReportView::Returned { state, effects } => {
                self.add("TurnReportView::Returned");
                self.add(match state {
                    StateReportView::Unchanged {} => "StateReportView::Unchanged",
                    StateReportView::Set { value: Value::Null } => "StateReportView::Set(null)",
                    StateReportView::Set { .. } => "StateReportView::Set",
                    StateReportView::Stop {} => "StateReportView::Stop",
                });
                for send in &effects.sends {
                    self.add(match (send.deliver_at, send.delay) {
                        (None, None) => "TurnSendView(now)",
                        (Some(_), None) => "TurnSendView(deliverAt)",
                        (None, Some(_)) => "TurnSendView(delay)",
                        (Some(_), Some(_)) => "TurnSendView(both)",
                    });
                    send.retry.iter().for_each(|r| self.retry(r));
                    send.retention.iter().for_each(|r| self.retention(r));
                }
                for create in &effects.creates {
                    self.add(if create.child {
                        "TurnEffectsView.creates(child)"
                    } else {
                        "TurnEffectsView.creates(independent)"
                    });
                }
                if !effects.cancels.is_empty() {
                    self.add("TurnEffectsView.cancels");
                }
                for watch in &effects.watches {
                    self.add(match watch.target {
                        WatchTarget::Actor(_) => "WatchTarget::Actor",
                        WatchTarget::Types(_) => "WatchTarget::Types",
                        WatchTarget::Every => "WatchTarget::Every",
                    });
                }
                for control in &effects.controls {
                    self.add(match control.effect {
                        ControlEffectView::Pause => "ControlEffectView::Pause",
                        ControlEffectView::Resume => "ControlEffectView::Resume",
                        ControlEffectView::Purge => "ControlEffectView::Purge",
                        ControlEffectView::Delete => "ControlEffectView::Delete",
                    });
                }
            }
            TurnReportView::Failed { cause, .. } => {
                self.add("TurnReportView::Failed");
                self.cause(*cause);
            }
            TurnReportView::Cancelled {} => self.add("TurnReportView::Cancelled"),
        }
    }
}

fn decode<T: DeserializeOwned>(text: &str) -> Result<T, (Stage, String)> {
    serde_json::from_str(text).map_err(|e| (Stage::Decode, e.to_string()))
}

fn encode<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

/// The body `text` of `operation` and `direction` decoded by the typed wire decoder, converted to the core's type and
/// back, and encoded again; or the stage that refused it. The release response has no body.
fn exchange(operation: &str, direction: &str, text: &str, seen: &mut Seen) -> Result<String, (Stage, String)> {
    let conversion = |e: cicada::api::RequestError| (Stage::Conversion, e.to_string());
    Ok(match (operation, direction) {
        ("lease", "request") => encode(&decode::<LeaseTurnsRequest>(text)?),
        ("renew", "request") => encode(&decode::<RenewTurnRequest>(text)?),
        ("release", "request") => encode(&decode::<ReleaseTurnRequest>(text)?),
        ("complete", "request") => {
            let request = decode::<CompleteTurnRequest>(text)?;
            seen.report(&request.report);
            let report = TurnReport::try_from(request.report.clone()).map_err(conversion)?;
            encode(&CompleteTurnRequest { report: report.into(), ..request })
        }
        ("lease", "response") => {
            let response = decode::<LeaseTurnsResponse>(text)?;
            seen.lease(&response);
            encode(&LeaseTurnsResponse::from(LeaseTurnsResult::from(response)))
        }
        ("renew", "response") => {
            let response = decode::<RenewTurnResponse>(text)?;
            seen.renew(&response);
            encode(&RenewTurnResponse::from(RenewTurnResult::from(response)))
        }
        ("complete", "response") => {
            let response = decode::<CompleteTurnResponse>(text)?;
            seen.complete(&response);
            encode(&CompleteTurnResponse::from(CompleteTurnResult::try_from(response).map_err(conversion)?))
        }
        other => panic!("no body for {other:?}"),
    })
}

const OPERATIONS: [&str; 4] = ["lease", "renew", "release", "complete"];

/// The operation and direction of a lease case, checked.
fn route(case: &Value) -> (&str, &str) {
    let (operation, direction) = (text(case, "operation"), text(case, "direction"));
    assert!(OPERATIONS.contains(&operation), "{}: operation {operation}", case["name"]);
    assert!(["request", "response"].contains(&direction), "{}: direction {direction}", case["name"]);
    (operation, direction)
}

/// conformance-fixtures, Scenario: Complete lease examples and normalization (valid cases).
#[test]
fn every_valid_lease_body_decodes_converts_and_encodes_to_its_exact_bytes() {
    let file = fixture("leases.json");
    let mut seen = Seen::default();
    for case in cases(&file, "valid") {
        let name = &case["name"];
        let (operation, direction) = route(case);
        match (operation, direction) {
            ("release", "response") => {
                assert_eq!(case["status"], 204, "{name}");
                assert!(case["body"].is_null(), "{name}: the 204 response has no body");
                continue;
            }
            (_, "request") => assert!(case["status"].is_null(), "{name}: a request has no status"),
            _ => assert_eq!(case["status"], 200, "{name}"),
        }
        let body = text(case, "body");
        let again = exchange(operation, direction, body, &mut seen).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(again, body, "{name}: Rust's output");
    }
    let required: BTreeSet<&str> = VARIANTS.iter().copied().collect();
    assert_eq!(seen.0, required, "the valid lease cases show exactly the listed variants");
}

/// conformance-fixtures, Scenario: Complete lease examples and normalization (accepted inputs).
#[test]
fn every_accepted_lease_input_normalizes_to_its_named_valid_case() {
    let file = fixture("leases.json");
    let valid = cases(&file, "valid");
    for case in cases(&file, "accepted") {
        let name = &case["name"];
        let (operation, direction) = route(case);
        let expected = text(case, "expected");
        let target = valid.iter().find(|v| v["name"] == expected).unwrap_or_else(|| panic!("{name}: no {expected}"));
        assert_eq!(route(target), (operation, direction), "{name}: {expected} is another body");
        assert_ne!(text(case, "input"), text(target, "body"), "{name}: the input is not already normalized");
        let normalized = exchange(operation, direction, text(case, "input"), &mut Seen::default())
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(normalized, text(target, "body"), "{name}: normalized output");
    }
}

/// conformance-fixtures, Scenario: Refused wire data.
#[test]
fn every_invalid_lease_input_fails_at_its_declared_stage() {
    let file = fixture("leases.json");
    let mut stages = BTreeSet::new();
    for case in cases(&file, "invalid") {
        let name = &case["name"];
        let (operation, direction) = route(case);
        let stage = Stage::parse(text(case, "stage"));
        let result = exchange(operation, direction, text(case, "input"), &mut Seen::default());
        let (found, why) = result.expect_err(&format!("{name}: accepted"));
        assert_eq!(found, stage, "{name}: refused at {found:?} ({why})");
        stages.insert(format!("{stage:?}"));
    }
    assert_eq!(stages.len(), 2, "both stages have cases");
}

// ---- the set of files ----

/// conformance-fixtures, Requirement: Fixed fixtures are checked against the implementation (membership, format,
/// unique names and references across the four files).
#[test]
fn the_fixtures_are_four_format_1_files_with_unique_names_and_resolving_references() {
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(dir()).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file(), "{:?} is a regular file", entry.path());
        found.insert(entry.file_name().into_string().unwrap());
    }
    let expected: BTreeSet<String> =
        ["canonical.json", "contracts.json", "leases.json", "status.json"].map(String::from).into();
    assert_eq!(found, expected, "fixtures/ holds exactly the four files");

    let canonical = fixture("canonical.json");
    cases(&canonical, "valid");
    cases(&canonical, "invalid");
    cases(&fixture("contracts.json"), "cases");
    let leases = fixture("leases.json");
    let status = fixture("status.json");
    let success = status["success"].as_object().unwrap();
    for list in ["valid", "accepted", "invalid"] {
        for case in cases(&leases, list) {
            let (operation, direction) = route(case);
            assert!(success.contains_key(operation), "{}: status.json has no {operation}", case["name"]);
            if list == "valid" && direction == "response" {
                assert_eq!(case["status"], success[operation]["status"], "{}: its success status", case["name"]);
            }
        }
    }
    let valid = names(cases(&leases, "valid"));
    for case in cases(&leases, "accepted") {
        assert!(valid.contains(text(case, "expected")), "{}: expected names a valid case", case["name"]);
    }
    for operation in OPERATIONS {
        assert!(
            cases(&leases, "valid").iter().any(|c| c["operation"] == operation && c["direction"] == "request"),
            "a {operation} request"
        );
        assert!(
            cases(&leases, "valid").iter().any(|c| c["operation"] == operation && c["direction"] == "response"),
            "a {operation} response"
        );
    }
    for row in cases(&status, "registration") {
        assert!(["invalidContract", "conflict", "invalid", "unavailable", "fatal"].contains(&text(row, "result")));
    }
}
