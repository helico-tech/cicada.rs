//! The contracts this service registers, against the snapshots in `tests/contracts`. A
//! change to any of them fails here, before a deploy would fail at node start next to nodes of the previous version.

use std::collections::BTreeMap;
use std::path::Path;

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada_examples::supervision::{MailerActor, Smtp, SmtpError};
use cicada_sample_service::Catalog;
use cicada_sample_service::e2e::EffectLog;
use sha2::{Digest, Sha256};

fn catalog_contracts() -> Vec<Contract> {
    let catalog = Catalog::new("contracts", EffectLog::new());
    catalog.types().iter().map(|t| t.contract().unwrap_or_else(|p| panic!("{}: {p}", t.type_name()))).collect()
}

#[test]
fn contracts_match_their_snapshots() {
    cicada_test::assert_contracts_match(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/contracts"), &catalog_contracts());
}

/// A mail server for the contract of `mailers`, the one example this service does not host.
struct NoSmtp;

impl Smtp for NoSmtp {
    async fn send(&self, _to: &str) -> Result<(), SmtpError> {
        Ok(())
    }
}

/// Scenario: A type that is not hit (schema-reference-names). Every contract of the examples, the ready-made actors and
/// this service is the text `GET /types` served at 0.14.1, byte for byte, its descriptions and the order of its keys
/// included: `tests/contract-texts.json` holds the SHA-256 of each text. The snapshots compare hashes without the
/// descriptions. `CICADA_UPDATE_CONTRACTS=true` rewrites the file.
#[test]
fn contracts_are_the_texts_recorded() {
    let mut contracts = catalog_contracts();
    contracts.push(contract_of::<MailerActor<NoSmtp>>().unwrap());
    let current: BTreeMap<String, String> = contracts
        .iter()
        .map(|c| {
            let digest = Sha256::digest(serde_json::to_string(&c.to_json()).unwrap().as_bytes());
            (c.type_name().to_string(), digest.iter().map(|b| format!("{b:02x}")).collect())
        })
        .collect();
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contract-texts.json");
    if std::env::var(cicada_test::UPDATE_CONTRACTS_ENV).is_ok_and(|v| v == "true") {
        std::fs::write(&file, serde_json::to_string_pretty(&current).unwrap() + "\n").unwrap();
        return;
    }
    let recorded: BTreeMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).expect("a map of type names to SHA-256");
    let differ: Vec<&String> =
        current.keys().chain(recorded.keys()).filter(|t| current.get(*t) != recorded.get(*t)).collect();
    assert!(differ.is_empty(), "The text of these contracts differs from {}: {differ:?}", file.display());
}
