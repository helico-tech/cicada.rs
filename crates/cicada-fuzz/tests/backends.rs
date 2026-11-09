//! `InMemoryStore` and `SqliteStore` give equivalent answers to the same operations, and the comparison itself works.

use std::collections::{BTreeMap, BTreeSet};

use cicada::address::ActorAddress;
use cicada::store::{Claim, OutgoingMessage, StoreError};
use cicada_fuzz::compare::{Op, Pair, Ranker, error_kind, in_address_order, op, ops};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

/// How many lists a normal `cargo test` runs; `PROPTEST_CASES` overrides it.
const DEFAULT_CASES: u32 = 256;

fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_CASES);
    ProptestConfig { cases, ..ProptestConfig::default() }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn backends_agree(list in ops()) {
        let result = block_on(async { Pair::memory_and_sqlite().await.run(&list).await });
        if let Err(difference) = result {
            return Err(TestCaseError::fail(difference.to_string()));
        }
    }
}

#[test]
fn generated_lists_reach_every_operation() {
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    for _ in 0..5_000 {
        seen.insert(op().new_tree(&mut runner).unwrap().current().name());
    }
    assert_eq!(seen, BTreeSet::from(Op::ALL));
}

#[tokio::test]
async fn a_changed_answer_from_one_backend_is_a_difference() {
    let mut pair = Pair::memory_twice().await;
    let get = Op::Get { to: 5 };
    pair.step(0, &get).await.expect("both have no ghost/0 yet");
    pair.second.apply(&Op::Register { ty: 2, variant: 0 }, cicada_test::default_start()).await;
    pair.second.apply(&Op::Create { to: 5, state: 1 }, cicada_test::default_start()).await;
    let difference = pair.step(1, &get).await.expect_err("only the second has it");
    assert!(difference.place.contains("operation 1"), "{difference}");
    assert_eq!(difference.first, serde_json::Value::Null);
}

#[tokio::test]
async fn a_difference_in_the_final_state_is_reported() {
    let mut pair = Pair::memory_twice().await;
    pair.compare_final_state().await.expect("equal at the start");
    pair.first.apply(&Op::Delete { to: 0 }, cicada_test::default_start()).await;
    let difference = pair.compare_final_state().await.expect_err("one lost an actor");
    assert_eq!(difference.place, "final state");
}

#[test]
fn numbers_compare_by_rank_and_not_by_value() {
    let tokens = |values: &[i64]| {
        let mut ranker = Ranker::default();
        values.iter().map(|v| ranker.token(*v)).collect::<Vec<_>>()
    };
    assert_eq!(tokens(&[5, 9, 5, 7]), tokens(&[100, 300, 100, 200]), "same order: equal");
    assert_ne!(tokens(&[5, 9]), tokens(&[9, 5]), "another order: different");
    assert_ne!(tokens(&[5, 5]), tokens(&[5, 6]), "repeated against new: different");
}

#[test]
fn errors_compare_by_kind_and_not_by_text() {
    assert_eq!(error_kind(&StoreError::Invalid("one".into())), error_kind(&StoreError::Invalid("two".into())));
    assert_eq!(error_kind(&StoreError::unavailable("one")), error_kind(&StoreError::unavailable("two")));
    assert_ne!(error_kind(&StoreError::Invalid("x".into())), error_kind(&StoreError::Corrupt("x".into())));
    assert_ne!(error_kind(&StoreError::unavailable("x")), error_kind(&StoreError::Corrupt("x".into())));
}

#[test]
fn claims_compare_in_address_order() {
    let claim = |text: &str| {
        let actor = ActorAddress::parse(text).unwrap();
        let at = cicada_test::default_start();
        let message = OutgoingMessage::new(actor.clone(), "M", serde_json::json!({})).into_message(at, 1);
        Claim { actor, epoch: 1, message, state: None, version: 0, claimed_at: at, lease_until: at }
    };
    let order = |list: Vec<Claim>| in_address_order(list).iter().map(|c| c.actor.to_string()).collect::<Vec<_>>();
    assert_eq!(order(vec![claim("b/1"), claim("a/1")]), order(vec![claim("a/1"), claim("b/1")]));
}

#[tokio::test]
async fn each_backend_pages_with_its_own_cursors() {
    // Page size 1 forces a cursor on every page; the seq in the cursors differs per store, the pages do not.
    let mut pair = Pair::memory_twice().await;
    let list = Op::ListPending { prefix: 0, limit: 1 };
    pair.step(0, &list).await.expect("paging agrees");
    let cursor = Op::ProbeCursor { listing: 2, cursor: 1, limit: 1 };
    pair.step(1, &cursor).await.expect("both refuse a bad cursor alike");
}

/// Passing means little if lists stop at refusals. The floors are well below what lists reach today.
#[tokio::test]
async fn generated_lists_reach_deep_states() {
    let marks = [
        ("claims", "\"epoch\"", 1_000),
        ("commits", "\"committed\"", 300),
        ("sends from a commit", "\"sent\":[{", 150),
        ("cancels from a commit", "\"cancelled\":[\"", 10),
        ("lost leases", "lease-lost", 300),
        ("rejected commits", "\"rejected\"", 200),
        ("duplicates", "\"duplicate\"", 200),
        ("retried messages", "\"failures\":1", 80),
        ("messages being run", "\"running\":{", 40),
        ("finished messages", "\"finished\":{", 100),
        ("cancel requests", "cancel-requested", 5),
        ("refused requests", "\"error\":\"invalid\"", 300),
    ];
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut runner = TestRunner::deterministic();
    for _ in 0..400 {
        let list = ops().new_tree(&mut runner).unwrap().current();
        let mut pair = Pair::memory_twice().await;
        for op in &list {
            if let Op::Advance(by) = op {
                pair.clock.advance_by(*by);
                continue;
            }
            let now = cicada::store::Clock::now(&*pair.clock);
            let answer = pair.first.apply(op, now).await.to_string();
            for (name, needle, _) in marks {
                *counts.entry(name).or_default() += answer.matches(needle).count();
            }
        }
    }
    for (name, _, floor) in marks {
        let seen = counts[name];
        assert!(seen >= floor, "{name}: reached {seen} times, expected at least {floor}\n{counts:#?}");
    }
}
