//! `InMemoryStore` and `SqliteStore` give equivalent answers to the same operations, and the comparison itself works.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use cicada::address::ActorAddress;
use cicada::store::{Claim, OutgoingMessage, RenewResult, Store, StoreError};
use cicada_fuzz::compare::{
    self, ADDRESSES, ALL_DUE, Commit, Due, End, Op, Pair, Ranker, StateOp, error_kind, in_address_order, op, ops,
};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;
use serde_json::{Value, json};

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

fn send(to: usize, id: usize, due: Due) -> compare::Send {
    compare::Send { to, id, due, payload: 0, from: None, caused_by: None, retry: false, retention: 0, extras: false }
}

fn commit(claim: usize, epoch_delta: i64) -> Commit {
    Commit {
        claim,
        epoch_delta,
        wrong_message: None,
        end: End::Done,
        state: StateOp::Unchanged,
        creates: vec![],
        sends: vec![],
        cancels: vec![],
    }
}

fn claim_all(ttl: Duration) -> Op {
    Op::ClaimDue { types: vec![0, 1], node: 0, ttl, limit: ALL_DUE }
}

const NS: Duration = Duration::from_nanos(1);
const US: Duration = Duration::from_micros(1);

/// The list `backends_agree` found in the CI of 0.3.1, shrunk. `alpha/0` and `alpha/1` are due at one instant and the
/// backends hand out their epochs in another order; `epoch_delta: -1` then closed different claims in the two.
#[tokio::test]
async fn the_list_with_two_instances_due_at_one_instant_agrees() {
    let list = [
        Op::Enqueue(send(1, 18, Due::After(Duration::ZERO))),
        claim_all(US),
        Op::Enqueue(send(0, 0, Due::Now)),
        Op::Delete { to: 0 },
        Op::Enqueue(send(0, 0, Due::Now)),
        Op::Enqueue(send(0, 0, Due::Now)),
        Op::Commit(Commit {
            end: End::Rescheduled(Due::AtAhead(NS)),
            creates: vec![(0, 0)],
            sends: vec![send(0, 1, Due::After(NS))],
            ..commit(0, 0)
        }),
        Op::Advance(US),
        claim_all(US),
        Op::SetPaused { to: 1, paused: true },
        Op::Advance(US),
        claim_all(US),
        Op::Commit(commit(0, -1)),
        Op::Commit(commit(2, 0)),
    ];
    Pair::memory_and_sqlite().await.run(&list).await.unwrap_or_else(|difference| panic!("{difference}"));
}

/// The epoch of the lease `address` holds in `store`: the small number that renews it. Renewed with the claim's ttl at
/// the claim's instant, the lease stays as it was.
async fn lease_epoch(store: &dyn Store, address: usize, ttl: Duration) -> i64 {
    let address = ActorAddress::parse(ADDRESSES[address]).unwrap();
    for epoch in 1..=8 {
        if let Ok(RenewResult::Renewed(_)) = store.renew(&address, epoch, ttl).await {
            return epoch;
        }
    }
    panic!("{address} holds no lease")
}

/// Both backends after `alpha/0` and `alpha/1` were due at one instant and claimed together, each backend handing
/// out their epochs in its own order, and after `alpha/0` alone was claimed again. The open claims, counted back from
/// the latest: 0 is the second claim of `alpha/0`, 1 the claim of `alpha/1`, 2 the first claim of `alpha/0`.
async fn tied_pair() -> Pair {
    async fn run(pair: &mut Pair, ops: &[Op]) {
        for (number, op) in ops.iter().enumerate() {
            pair.step(number, op).await.unwrap_or_else(|difference| panic!("{difference}"));
        }
    }
    let mut pair = Pair::memory_and_sqlite().await;
    let to_one_instant = [
        Op::Enqueue(send(1, 18, Due::After(Duration::ZERO))),
        claim_all(US),
        Op::Delete { to: 0 },
        // `alpha/1` goes back to its mailbox, and a re-created `alpha/0` gets a message: both are due 1 ns ahead.
        Op::Commit(Commit {
            end: End::Rescheduled(Due::AtAhead(NS)),
            creates: vec![(0, 0)],
            sends: vec![send(0, 1, Due::After(NS))],
            ..commit(0, 0)
        }),
        Op::Advance(US),
        claim_all(US),
    ];
    run(&mut pair, &to_one_instant).await;
    let mut orders = Vec::new();
    for store in [&pair.first.store, &pair.second.store] {
        orders.push(lease_epoch(&**store, 0, US).await < lease_epoch(&**store, 1, US).await);
    }
    assert_ne!(orders[0], orders[1], "the backends hand out the epochs of alpha/0 and alpha/1 in another order");
    run(&mut pair, &[Op::SetPaused { to: 1, paused: true }, Op::Advance(US), claim_all(Duration::from_secs(30))]).await;
    pair
}

/// Runs `op` on both backends and returns the answer they agree on.
async fn agreed(pair: &mut Pair, op: Op) -> Value {
    let now = cicada::store::Clock::now(&*pair.clock);
    let first = pair.first.apply(&op, now).await;
    let second = pair.second.apply(&op, now).await;
    assert_eq!(first, second, "backends differ at {op:?}");
    first
}

async fn finish(pair: Pair) {
    pair.run(&[]).await.unwrap_or_else(|difference| panic!("{difference}"));
}

#[tokio::test]
async fn after_a_tie_a_commit_reaches_the_same_claim_in_both_backends() {
    // -1 from the second claim of alpha/0 is its first: stale, and closed in both.
    let mut pair = tied_pair().await;
    assert_eq!(agreed(&mut pair, Op::Commit(commit(0, -1))).await, json!("lease-lost"));
    // So alpha/1 and the second claim of alpha/0 are open, and 2 back is the second claim of alpha/0.
    assert!(agreed(&mut pair, Op::Commit(commit(2, 0))).await.get("committed").is_some());
    finish(pair).await;

    // 1 from the first claim of alpha/0 is its second, which holds the lease.
    let mut pair = tied_pair().await;
    assert!(agreed(&mut pair, Op::Commit(commit(2, 1))).await.get("committed").is_some());
    finish(pair).await;
}

#[tokio::test]
async fn after_a_tie_a_renew_reaches_the_same_claim_in_both_backends() {
    let mut pair = tied_pair().await;
    let renew = |claim, epoch_delta| Op::Renew { claim, epoch_delta, ttl: Duration::from_secs(30) };
    // -1 from the second claim of alpha/0 is its first, which is stale; 1 from the first is the second, which is not.
    assert_eq!(agreed(&mut pair, renew(0, -1)).await, json!("lease-lost"));
    assert!(agreed(&mut pair, renew(2, 1)).await.get("renewed").is_some());
    finish(pair).await;
}

#[tokio::test]
async fn after_a_tie_a_release_reaches_the_same_claim_in_both_backends() {
    // -1 from the second claim of alpha/0 releases nothing and closes its first claim in both.
    let mut pair = tied_pair().await;
    assert_eq!(agreed(&mut pair, Op::Release { claim: 0, epoch_delta: -1 }).await, json!("released"));
    assert!(agreed(&mut pair, Op::Commit(commit(2, 0))).await.get("committed").is_some());
    finish(pair).await;

    // 1 from the first claim of alpha/0 releases the lease of its second.
    let mut pair = tied_pair().await;
    assert_eq!(agreed(&mut pair, Op::Release { claim: 2, epoch_delta: 1 }).await, json!("released"));
    assert_eq!(agreed(&mut pair, Op::RunningTurn { to: 0 }).await, Value::Null);
    finish(pair).await;
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
