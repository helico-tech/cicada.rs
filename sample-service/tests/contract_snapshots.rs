//! The contracts this service registers, against the snapshots in `tests/contracts`. A
//! change to any of them fails here, before a deploy would fail at node start next to nodes of the previous version.

use cicada_sample_service::e2e::EffectLog;
use cicada_sample_service::{Catalog, GROUPS};

#[test]
fn contracts_match_their_snapshots() {
    let groups: Vec<String> = GROUPS.iter().map(|g| (*g).to_owned()).collect();
    let catalog = Catalog::new("contracts", EffectLog::new());
    let contracts: Vec<_> = catalog
        .types(&groups)
        .iter()
        .map(|t| t.contract().unwrap_or_else(|p| panic!("{}: {p}", t.type_name())))
        .collect();
    cicada_test::assert_contracts_match(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/contracts"), &contracts);
}
