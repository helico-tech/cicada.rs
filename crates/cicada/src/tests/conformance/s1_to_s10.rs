//! The S1–S10 suite, plus the type registry, instance and query operations the
//! runtime and HTTP API rely on.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::address::ActorAddress;
use crate::retention::Retention;
use crate::store::{
    ActorCreate, Backend, CancelResult, Claim, CommitRequest, CommitResult, CreateResult, DeleteResult, Delivery,
    EnqueueResult, Message, MessageId, MessageStatus, OutcomeRecord, OutcomeStatus, OutgoingMessage, Page, PageCursor,
    RegisterResult, RemoveTypeResult, RenewResult, StateChange, StoreError, StoreStats, TurnOutcome, TypeDef,
};
use crate::time::Timestamp;
use serde_json::json;

use crate::tests::conformance::env::*;
use crate::tests::conformance::fixture::{FixtureFactory, StoreFixture};

const RETENTION_TICKS: u32 = 10;

fn tick_retention<B: FixtureFactory>(backend: &B) -> Duration {
    backend.tick() * RETENTION_TICKS
}

const PAGE: usize = Page::<Message>::DEFAULT_SIZE;

async fn list_types(store: &dyn Backend) -> Vec<TypeDef> {
    store.list_types().await.unwrap()
}

async fn outcome_status(store: &dyn Backend, id: &MessageId) -> Option<OutcomeStatus> {
    outcome_record(store, id).await.map(|o| o.status)
}

async fn state_of(store: &dyn Backend, a: &ActorAddress) -> serde_json::Value {
    store.get(a).await.unwrap().expect("the instance").state
}

async fn version_of(store: &dyn Backend, a: &ActorAddress) -> i64 {
    store.get(a).await.unwrap().expect("the instance").version
}

/// The definitions in `types` rotated by `by` (and reversed for odd `by`), so concurrent registrations use different
/// orders.
fn reordered(types: &[TypeDef], by: usize) -> Vec<TypeDef> {
    let mut out = types.to_vec();
    out.rotate_left(by % types.len().max(1));
    if by % 2 == 1 {
        out.reverse();
    }
    out
}

// ------------------------------------------------------------------------------------------------------------------
// TypeRegistry
// ------------------------------------------------------------------------------------------------------------------

/// registry - register is idempotent and types are listed by name
pub async fn registry_register_is_idempotent_and_types_are_listed_by_name<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let b = type_def(&type_name("b"), "h1");
        let a = type_def(&type_name("a"), "h1");
        assert_eq!(store.register(b.clone()).await.unwrap(), RegisterResult::Registered(b.clone()));
        assert_eq!(store.register(a.clone()).await.unwrap(), RegisterResult::Registered(a.clone()));
        assert_eq!(store.register(b.clone()).await.unwrap(), RegisterResult::Registered(b.clone()));
        assert_eq!(list_types(&*store).await, vec![a, b]);
    })
    .await
}

/// registry - same name with a different contract hash conflicts
pub async fn registry_same_name_with_a_different_contract_hash_conflicts<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let v1 = type_def(&TYPE, "h1");
        store.register(v1.clone()).await.unwrap();
        let existing = expect_matches!(
            store.register(type_def(&TYPE, "h2")).await.unwrap(),
            RegisterResult::Conflict { existing, .. } => existing
        );
        assert_eq!(existing, v1);
        assert_eq!(list_types(&*store).await, vec![v1]);
    })
    .await
}

/// registry - nodes registering the same contracts concurrently all succeed
pub async fn registry_nodes_registering_the_same_contracts_concurrently_all_succeed<B: FixtureFactory>(backend: &B) {
    // Every node of a deployment registers the same types at start, often at the same moment.
    store_test(backend, async |t| {
        for round in 0..5 {
            let types: Arc<Vec<TypeDef>> =
                Arc::new((0..8).map(|i| type_def(&type_name(&format!("concurrent-{round}.t{i}")), "h1")).collect());
            let results: Vec<RegisterResult> = t
                .concurrently(8, |i, peer| {
                    let types = types.clone();
                    async move {
                        let mut out = Vec::new();
                        for d in reordered(&types, i) {
                            out.push(peer.register(d).await.unwrap());
                        }
                        out
                    }
                })
                .await
                .into_iter()
                .flatten()
                .collect();
            let conflicts: Vec<_> = results.iter().filter(|r| matches!(r, RegisterResult::Conflict { .. })).collect();
            assert_eq!(conflicts, Vec::<&RegisterResult>::new());
        }
    })
    .await
}

/// registry - remove_type removes an unused type, refuses one with instances and reports an absent one
pub async fn registry_remove_type_removes_an_unused_type_refuses_one_with_instances_and_reports_an_absent_one<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        assert_eq!(store.remove_type(&TYPE).await.unwrap(), RemoveTypeResult::Absent);
        let used = type_def(&TYPE, "h1");
        let unused = type_def(&OTHER, "h1");
        // A type whose name is a prefix of another one's name does not see that one's instances.
        let short = type_def(&type_name("conf"), "h1");
        for def in [&used, &unused, &short] {
            store.register(def.clone()).await.unwrap();
        }
        let a = t.new_actor(&TYPE, json!({})).await;
        assert_eq!(store.remove_type(&TYPE).await.unwrap(), RemoveTypeResult::InUse);
        assert_eq!(list_types(&*store).await, vec![short.clone(), used.clone(), unused.clone()]);
        assert_eq!(store.remove_type(&OTHER).await.unwrap(), RemoveTypeResult::Removed);
        assert_eq!(store.remove_type(&type_name("conf")).await.unwrap(), RemoveTypeResult::Removed);
        assert_eq!(store.remove_type(&OTHER).await.unwrap(), RemoveTypeResult::Absent);
        assert_eq!(list_types(&*store).await, vec![used.clone()]);

        // Once its last instance is gone the type can go, and be registered again with another contract.
        assert!(matches!(store.delete(&a, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. }));
        assert_eq!(store.remove_type(&TYPE).await.unwrap(), RemoveTypeResult::Removed);
        assert!(list_types(&*store).await.is_empty());
        let changed = type_def(&TYPE, "h2");
        assert_eq!(store.register(changed.clone()).await.unwrap(), RegisterResult::Registered(changed));
    })
    .await
}

/// registry - nodes removing one type concurrently remove it once, and a racing create is either seen or finds it gone
pub async fn registry_nodes_removing_one_type_concurrently_remove_it_once_and_a_racing_create_is_not_lost<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..5 {
            let name = type_name(&format!("removal{round}"));
            store.register(type_def(&name, "h1")).await.unwrap();
            let results = t
                .concurrently(8, |_, peer| {
                    let name = name.clone();
                    async move { peer.remove_type(&name).await.unwrap() }
                })
                .await;
            assert_eq!(results.iter().filter(|r| **r == RemoveTypeResult::Removed).count(), 1, "{results:?}");
            assert!(results.iter().all(|r| matches!(r, RemoveTypeResult::Removed | RemoveTypeResult::Absent)));

            // A create racing a remove: when the remove answers `InUse`, the instance exists and the type stays; when
            // it answers `Removed`, the type is gone.
            store.register(type_def(&name, "h1")).await.unwrap();
            let target = address(&format!("{name}/x"));
            let results = t
                .concurrently(2, |i, peer| {
                    let (name, target) = (name.clone(), target.clone());
                    async move {
                        if i == 0 {
                            peer.create(&target, json!({})).await.unwrap();
                            None
                        } else {
                            Some(peer.remove_type(&name).await.unwrap())
                        }
                    }
                })
                .await;
            let removal = results.into_iter().flatten().next().expect("the remove answered");
            let registered = list_types(&*store).await.iter().any(|d| d.name == name);
            match removal {
                RemoveTypeResult::InUse => assert!(registered && store.get(&target).await.unwrap().is_some()),
                RemoveTypeResult::Removed => assert!(!registered),
                RemoveTypeResult::Absent => panic!("the type was registered"),
            }
        }
    })
    .await
}

/// registry - nodes registering one name with different contracts concurrently admit exactly one
pub async fn registry_nodes_registering_one_name_with_different_contracts_concurrently_admit_exactly_one<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        for round in 0..5 {
            let types: Arc<Vec<TypeDef>> = Arc::new(
                ["h1", "h2", "h3", "h4"]
                    .into_iter()
                    .map(|h| type_def(&type_name(&format!("race-{round}")), h))
                    .collect(),
            );
            let results = t
                .concurrently(types.len(), |i, peer| {
                    let types = types.clone();
                    async move { peer.register(types[i].clone()).await.unwrap() }
                })
                .await;
            assert_eq!(results.iter().filter(|r| matches!(r, RegisterResult::Registered(_))).count(), 1, "{results:?}");
            let name = format!("race-{round}");
            let registered = list_types(&*t.store()).await;
            assert_eq!(registered.iter().filter(|d| d.name.as_str() == name).count(), 1);
        }
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// StateStore
// ------------------------------------------------------------------------------------------------------------------

/// instances - create then get returns the initial snapshot
pub async fn instances_create_then_get_returns_the_initial_snapshot<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let before = store.now().await.unwrap();
        let a = t.new_actor(&TYPE, json!({"n": 1})).await;
        let after = store.now().await.unwrap();
        let rec = store.get(&a).await.unwrap().expect("the instance");
        assert_eq!(rec.address, a);
        assert_eq!(rec.address.type_name(), &*TYPE);
        assert_eq!(rec.state, json!({"n": 1}));
        assert_eq!(rec.version, 0);
        assert!(rec.created_at >= before && rec.created_at <= after);
        assert_eq!(store.get(&address("conformance/absent")).await.unwrap(), None);
        let b = t.new_actor(&TYPE, json!({})).await;
        assert_eq!(state_of(&*store, &b).await, json!({}));
    })
    .await
}

/// instances - create is idempotent and leaves the existing instance untouched
pub async fn instances_create_is_idempotent_and_leaves_the_existing_instance_untouched<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 1})).await;
        let m = t.send(&a).await;
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest { state: StateChange::NewState(json!({"n": 2})), ..done(&c) }).await;
        t.send(&a).await;
        let existing = expect_matches!(
            store.create(&a, json!({"n": 99})).await.unwrap(),
            CreateResult::Exists(existing) => existing
        );
        assert_eq!(existing.state, json!({"n": 2}));
        assert_eq!(existing.version, 1);
        assert_eq!(state_of(&*store, &a).await, json!({"n": 2}));
        assert_eq!(t.pending(&a).await.len(), 1);
        assert_eq!(outcome_status(&*store, &m.id).await, Some(OutcomeStatus::Done));
    })
    .await
}

/// instances - list by prefix in address order with paging
pub async fn instances_list_by_prefix_in_address_order_with_paging<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let mut wanted = Vec::new();
        for i in 1..=23 {
            wanted.push(t.new_actor_named(&TYPE, &format!("w{i:02}"), json!({})).await);
        }
        t.new_actor_named(&OTHER, "x", json!({})).await;
        t.new_actor_named(&type_name("conformance2"), "y", json!({})).await;
        let mut seen: Vec<ActorAddress> = Vec::new();
        let mut cursor: Option<PageCursor> = None;
        let mut pages = 0;
        loop {
            let page = store.list("conformance/", cursor.as_ref(), 5).await.unwrap();
            assert!(page.items.len() <= 5);
            seen.extend(page.items);
            pages += 1;
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        wanted.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        assert_eq!(seen, wanted);
        assert!((5..=6).contains(&pages), "{pages} pages");
        assert_eq!(store.list("", None, 100).await.unwrap().items.len(), 25);
        assert_eq!(store.list("other", None, PAGE).await.unwrap().items, vec![address("other/x")]);
        assert!(store.list("nothing/", None, PAGE).await.unwrap().items.is_empty());
    })
    .await
}

/// Every page of `list_records` (or of `list`, with `records` false) under `prefix` at `limit`: the addresses of each
/// page and the cursor that follows it.
async fn record_pages(
    store: &dyn Backend,
    prefix: &str,
    limit: usize,
    records: bool,
) -> Vec<(Vec<ActorAddress>, Option<PageCursor>)> {
    let mut pages = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let (items, next) = if records {
            let page = store.list_records(prefix, cursor.as_ref(), limit).await.unwrap();
            (page.items.into_iter().map(|r| r.address).collect(), page.next)
        } else {
            let page = store.list(prefix, cursor.as_ref(), limit).await.unwrap();
            (page.items, page.next)
        };
        pages.push((items, next.clone()));
        match next {
            Some(next) => cursor = Some(next),
            None => return pages,
        }
    }
}

/// instances - list records pages like list, with the snapshot get returns
pub async fn instances_list_records_pages_like_list_with_the_snapshot_get_returns<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let one = t.new_actor_named(&TYPE, "one", json!({"n": 0})).await;
        let two = t.new_actor_named(&TYPE, "two", json!({"deep": {"list": [1, 2, 3]}})).await;
        t.new_actor_named(&TYPE, "three", json!(null)).await;
        t.send(&one).await;
        let c = t.claim_one().await;
        t.commit_ok(CommitRequest {
            state: StateChange::NewState(json!({"n": 1})),
            creates: vec![ActorCreate { address: address("conformance/one-child"), state: json!({}), child: true }],
            ..done(&c)
        })
        .await;
        assert!(store.set_paused(&two, true).await.unwrap());
        t.new_actor_named(&OTHER, "x", json!({})).await;

        // Each row is the snapshot get returns, and the pages have the addresses and cursors of list.
        for limit in [1, 2, 3, 4, 5, 1001] {
            for prefix in ["conformance/", "", "conformance/t"] {
                assert_eq!(
                    record_pages(&*store, prefix, limit, true).await,
                    record_pages(&*store, prefix, limit, false).await,
                    "prefix '{prefix}', limit {limit}"
                );
            }
        }
        let page = store.list_records("conformance/", None, 1001).await.unwrap();
        assert_eq!(page.next, None);
        assert_eq!(page.items.len(), 4);
        for row in &page.items {
            assert_eq!(Some(row), store.get(&row.address).await.unwrap().as_ref());
        }
        let child = page.items.iter().find(|r| r.address.as_str() == "conformance/one-child").expect("the child");
        assert_eq!(child.parent.as_ref(), Some(&one));
        assert!(page.items.iter().any(|r| r.address == two && r.paused));
        assert!(page.items.iter().any(|r| r.address == one && r.version == 1 && r.state == json!({"n": 1})));

        // The last page has no cursor; nothing matching is an empty page with none.
        let first = store.list_records("conformance/", None, 2).await.unwrap();
        let last = store.list_records("conformance/", first.next.as_ref(), 2).await.unwrap();
        assert_eq!(last.items.len(), 2);
        assert_eq!(last.next, None);
        let none = store.list_records("nothing/", None, 2).await.unwrap();
        assert!(none.items.is_empty());
        assert_eq!(none.next, None);

        // A cursor before the prefix starts at the prefix; one past every match is an empty page.
        let before = store.list_records("other/", Some(&PageCursor("a".into())), 10).await.unwrap();
        assert_eq!(before.items.iter().map(|r| r.address.clone()).collect::<Vec<_>>(), vec![address("other/x")]);
        let after = store.list_records("conformance/", Some(&PageCursor("conformance/zzz".into())), 10).await.unwrap();
        assert!(after.items.is_empty());
        assert_eq!(after.next, None);
        for cursor in ["a", "conformance/zzz", "conformance/one"] {
            let cursor = Some(PageCursor(cursor.into()));
            let records = store.list_records("conformance/", cursor.as_ref(), 10).await.unwrap();
            let addresses = store.list("conformance/", cursor.as_ref(), 10).await.unwrap();
            assert_eq!(records.items.into_iter().map(|r| r.address).collect::<Vec<_>>(), addresses.items);
        }

        // Zero is refused, as by list; there is no upper cap.
        assert_matches!(store.list_records("", None, 0).await, Err(StoreError::Invalid(_)));
        assert_matches!(store.list("", None, 0).await, Err(StoreError::Invalid(_)));
    })
    .await
}

/// instances - list records takes the prefix literally, like list
pub async fn instances_list_records_takes_the_prefix_literally_like_list<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = type_name("a");
        let ab = type_name("ab");
        let mut all = [
            t.new_actor_named(&a, "one", json!({})).await,
            t.new_actor_named(&ab, "one", json!({})).await,
            t.new_actor_named(&a, "50%_x", json!({})).await,
            t.new_actor_named(&a, "5000", json!({})).await,
            t.new_actor_named(&a, "b_c", json!({})).await,
            t.new_actor_named(&a, "bxc", json!({})).await,
        ];
        all.sort_by(|x, y| x.as_str().cmp(y.as_str()));
        let listed = async |prefix: &str| {
            let records = store.list_records(prefix, None, 100).await.unwrap();
            let addresses = store.list(prefix, None, 100).await.unwrap();
            let records: Vec<String> = records.items.into_iter().map(|r| r.address.to_string()).collect();
            assert_eq!(records, addresses.items.iter().map(|a| a.to_string()).collect::<Vec<_>>(), "'{prefix}'");
            records
        };
        assert_eq!(listed("").await, all.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(listed("a").await.len(), 6, "a matches both types");
        assert_eq!(listed("a/").await.len(), 5, "a/ matches only type a");
        assert_eq!(listed("ab/").await, vec!["ab/one"]);
        // The canonical key of `50%_x` is `50%25_x`: percent and underscore are plain characters, not wildcards.
        assert_eq!(listed("a/50%").await, vec!["a/50%25_x"]);
        assert_eq!(listed("a/50%25_").await, vec!["a/50%25_x"]);
        assert_eq!(listed("a/b_").await, vec!["a/b_c"]);
        assert!(listed("a/%").await.is_empty());
        assert!(listed("a/_").await.is_empty());
    })
    .await
}

/// instances - delete of an absent instance reports absent
pub async fn instances_delete_of_an_absent_instance_reports_absent<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        assert_eq!(
            store.delete(&address("conformance/none"), &MessageId::random()).await.unwrap(),
            DeleteResult::Absent
        );
        let a = t.new_actor(&TYPE, json!({})).await;
        assert_eq!(store.delete(&a, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { purged: 0 });
        assert_eq!(store.delete(&a, &MessageId::random()).await.unwrap(), DeleteResult::Absent);
        assert_eq!(store.get(&a).await.unwrap(), None);
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S1 Durability
// ------------------------------------------------------------------------------------------------------------------

/// S1 - acknowledged writes survive a restart
pub async fn s1_acknowledged_writes_survive_a_restart<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !assume(t.fixture.supports_reopen(), "backend is not durable") {
            return;
        }
        t.store().register(type_def(&TYPE, "h1")).await.unwrap();
        let a = t.new_actor(&TYPE, json!({"n": 1})).await;
        let done_msg = t.send(&a).await;
        let c1 = t.claim_one().await;
        t.commit_ok(CommitRequest { state: StateChange::NewState(json!({"n": 2})), ..done(&c1) }).await;
        let pending_msg = t.send_at(&a, Delivery::After(t.ticks(100))).await;
        let b = t.new_actor(&TYPE, json!({})).await;
        let claimed_then_released = t.send(&b).await;
        let c2 = t.claim_one().await;
        t.store().release(&c2.actor, c2.epoch).await.unwrap();

        t.reopen().await;

        let store = t.store();
        assert_eq!(list_types(&*store).await, vec![type_def(&TYPE, "h1")]);
        let rec = store.get(&a).await.unwrap().expect("the instance");
        assert_eq!(rec.state, json!({"n": 2}));
        assert_eq!(rec.version, 1);
        assert_eq!(t.pending(&a).await, vec![pending_msg.clone()]);
        assert_eq!(outcome_status(&*store, &done_msg.id).await, Some(OutcomeStatus::Done));
        let status = store.status(&claimed_then_released.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Pending(m)) => m).attempt, 1);
        let c3 = t.claim_one().await;
        assert!(c3.epoch > c2.epoch, "epochs must stay monotonic across restarts");
        assert_eq!(c3.message.attempt, 2);
        assert!(t.send(&a).await.seq > pending_msg.seq, "seq must stay monotonic across restarts");
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S2 Atomicity
// ------------------------------------------------------------------------------------------------------------------

/// S2 - commit applies state, creates, sends and outcome together
pub async fn s2_commit_applies_state_creates_sends_and_outcome_together<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 0})).await;
        let other = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let claim = t.claim_one().await;
        let child = address("conformance/child");
        let committed = t
            .commit_ok(CommitRequest {
                state: StateChange::NewState(json!({"n": 1})),
                creates: vec![ActorCreate { address: child.clone(), state: json!({"child": 1}), child: false }],
                sends: vec![
                    OutgoingMessage {
                        from: Some(a.clone()),
                        correlation_id: Some("c1".into()),
                        headers: BTreeMap::from([("h".to_owned(), "v".to_owned())]),
                        ..OutgoingMessage::new(other.clone(), "Hello", json!("x"))
                    },
                    OutgoingMessage {
                        from: Some(a.clone()),
                        ..OutgoingMessage::new(child.clone(), "Init", json!("y"))
                    },
                ],
                ..done(&claim)
            })
            .await;
        assert_eq!(committed.version, 1);
        assert_eq!(committed.sent.len(), 2);
        let rec = store.get(&a).await.unwrap().expect("the instance");
        assert_eq!(rec.state, json!({"n": 1}));
        assert_eq!(rec.version, 1);
        assert_eq!(state_of(&*store, &child).await, json!({"child": 1}));
        let hello = t.pending(&other).await;
        assert_eq!(hello.len(), 1);
        let hello = &hello[0];
        assert_eq!(&committed.sent[0], hello);
        assert_eq!(hello.from.as_ref(), Some(&a));
        assert_eq!(hello.correlation_id.as_deref(), Some("c1"));
        assert_eq!(hello.headers, BTreeMap::from([("h".to_owned(), "v".to_owned())]));
        let init = t.pending(&child).await;
        assert_eq!(init.len(), 1);
        assert_eq!(init[0].message_type, "Init");
        let outcome = outcome_record(&*store, &m.id).await.expect("an outcome");
        assert_eq!(outcome.status, OutcomeStatus::Done);
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.envelope, claim.message);
        assert!(t.pending(&a).await.is_empty());
        // The lease is released: the next message is claimable at once.
        t.send(&a).await;
        assert!(t.claim().await.iter().any(|c| c.actor == a));
    })
    .await
}

/// S2 - a send to an absent instance rejects the whole commit and keeps the lease
pub async fn s2_a_send_to_an_absent_instance_rejects_the_whole_commit_and_keeps_the_lease<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 0})).await;
        let m = t.send(&a).await;
        let claim = t.claim_one_long().await;
        let child = address("conformance/child");
        let absent = address("conformance/absent");
        let result = t
            .commit(CommitRequest {
                state: StateChange::NewState(json!({"n": 1})),
                creates: vec![ActorCreate { address: child.clone(), state: json!({}), child: false }],
                sends: vec![
                    OutgoingMessage::new(child.clone(), "T", json!(1)),
                    OutgoingMessage::new(absent, "T", json!(2)),
                ],
                ..done(&claim)
            })
            .await;
        let reason = expect_matches!(result, CommitResult::Rejected(reason) => reason);
        assert!(!reason.trim().is_empty());
        // Nothing applied.
        assert_eq!(state_of(&*store, &a).await, json!({"n": 0}));
        assert_eq!(version_of(&*store, &a).await, 0);
        assert_eq!(store.get(&child).await.unwrap(), None);
        assert_eq!(outcome_record(&*store, &m.id).await, None);
        assert_eq!(t.pending(&a).await, vec![claim.message.clone()]);
        // Lease still held: not claimable, renewable, and a failed commit can follow.
        assert!(t.claim_node(&NODE_B).await.is_empty());
        assert_matches!(store.renew(&a, claim.epoch, t.ttl()).await.unwrap(), RenewResult::Renewed(_));
        t.commit_ok(ending(&claim, TurnOutcome::Failed(reason.clone()))).await;
        let outcome = outcome_record(&*store, &m.id).await.expect("an outcome");
        assert_eq!(outcome.status, OutcomeStatus::Failed);
        assert_eq!(outcome.error, Some(reason));
        assert_eq!(state_of(&*store, &a).await, json!({"n": 0}));
    })
    .await
}

/// S2 - creates are applied before sends in one commit
pub async fn s2_creates_are_applied_before_sends_in_one_commit<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let child = address("other/kid");
        t.register_type(&OTHER).await;
        let existing = t.new_actor(&TYPE, json!({"keep": 1})).await;
        let c = t.claim_one().await;
        let committed = t
            .commit_ok(CommitRequest {
                creates: vec![
                    ActorCreate { address: child.clone(), state: json!({"k": 1}), child: false },
                    ActorCreate { address: existing.clone(), state: json!({"keep": 2}), child: false },
                ],
                sends: vec![OutgoingMessage::new(child.clone(), "Start", json!(0))],
                ..done(&c)
            })
            .await;
        assert_eq!(committed.sent.len(), 1);
        assert_eq!(committed.sent[0].to, child);
        let rec = store.get(&child).await.unwrap().expect("the child");
        assert_eq!(rec.address.type_name(), &*OTHER);
        assert_eq!(rec.state, json!({"k": 1}));
        assert_eq!(state_of(&*store, &existing).await, json!({"keep": 1}), "create in a commit is idempotent");
        assert_eq!(t.claim_one_types(&[&OTHER]).await.message.message_type, "Start");
    })
    .await
}

/// S2 - a send reusing a message id rejects the commit
pub async fn s2_a_send_reusing_a_message_id_rejects_the_commit<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let finished = t.send(&b).await;
        t.commit_ok(done(&t.claim_one_types(&[&OTHER]).await)).await;
        let pending_b = t.send_at(&b, Delivery::After(t.ticks(50))).await;
        t.send(&a).await;
        let c = t.claim_one().await;
        let dup_id = MessageId::random();
        let candidates = [
            vec![dup_id.clone(), dup_id], // twice within one commit
            vec![finished.id.clone()],    // a retained outcome
            vec![pending_b.id.clone()],   // a pending message
            vec![c.message.id.clone()],   // the message being committed
        ];
        for message_ids in candidates {
            let sends = message_ids
                .iter()
                .enumerate()
                .map(|(i, id)| OutgoingMessage { id: id.clone(), ..OutgoingMessage::new(b.clone(), "T", json!(i)) })
                .collect();
            assert_matches!(
                t.commit(CommitRequest { sends, ..done(&c) }).await,
                CommitResult::Rejected(_),
                "ids {message_ids:?}"
            );
        }
        assert_eq!(t.pending(&b).await, vec![pending_b]);
        t.commit_ok(done(&c)).await;
    })
    .await
}

/// S2 - delete removes the instance and purges its whole mailbox
pub async fn s2_delete_removes_the_instance_and_purges_its_whole_mailbox<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 1})).await;
        let keep = t.new_actor(&OTHER, json!({})).await;
        let kept = t.send(&keep).await;
        let m1 = t.send(&a).await;
        let m2 = t.send_at(&a, Delivery::After(t.ticks(50))).await;
        let claim = t.claim_one().await;
        assert_eq!(store.delete(&a, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { purged: 2 });
        assert_eq!(store.get(&a).await.unwrap(), None);
        assert!(t.pending(&a).await.is_empty());
        for m in [&m1, &m2] {
            let o = outcome_record(&*store, &m.id).await.expect("an outcome");
            assert_eq!(o.status, OutcomeStatus::Purged);
            assert_eq!(o.actor, a);
            assert_eq!(o.envelope.id, m.id);
        }
        assert_eq!(
            outcome_record(&*store, &m1.id).await.expect("an outcome").envelope.attempt,
            1,
            "the envelope keeps the claim's attempt"
        );
        assert_eq!(
            store.enqueue(OutgoingMessage::new(a.clone(), "T", json!(1))).await.unwrap(),
            EnqueueResult::NoSuchInstance
        );
        assert_eq!(t.commit(done(&claim)).await, CommitResult::LeaseLost);
        assert_eq!(store.renew(&a, claim.epoch, t.ttl()).await.unwrap(), RenewResult::LeaseLost);
        assert_eq!(t.pending(&keep).await, vec![kept], "other mailboxes are untouched");
        assert_eq!(store.next_due_at(&type_set(&[&TYPE])).await.unwrap(), None);
    })
    .await
}

/// S2 - stop removes the instance and purges the rest of its mailbox
pub async fn s2_stop_removes_the_instance_and_purges_the_rest_of_its_mailbox<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 1})).await;
        let other = t.new_actor(&OTHER, json!({})).await;
        let current = t.send(&a).await;
        let later = t.send_at(&a, Delivery::After(t.ticks(50))).await;
        let claim = t.claim_one().await;
        let committed = t
            .commit_ok(CommitRequest {
                state: StateChange::Stop,
                sends: vec![
                    OutgoingMessage::new(a.clone(), "Self", json!(1)),
                    OutgoingMessage::new(other.clone(), "Bye", json!(2)),
                ],
                ..done(&claim)
            })
            .await;
        assert_eq!(store.get(&a).await.unwrap(), None);
        assert_eq!(outcome_status(&*store, &current.id).await, Some(OutcomeStatus::Done));
        assert_eq!(outcome_status(&*store, &later.id).await, Some(OutcomeStatus::Purged));
        assert_eq!(
            outcome_status(&*store, &committed.sent[0].id).await,
            Some(OutcomeStatus::Purged),
            "self-sends of a stopping turn are purged"
        );
        assert_eq!(t.pending(&other).await, vec![committed.sent[1].clone()]);
        assert!(t.claim().await.is_empty());
        // Re-create starts from scratch.
        assert_eq!(store.create(&a, json!({})).await.unwrap(), CreateResult::Created);
        assert_eq!(version_of(&*store, &a).await, 0);
        assert!(t.pending(&a).await.is_empty());
    })
    .await
}

/// S2 - a failed outcome keeps state and records the error and envelope
pub async fn s2_a_failed_outcome_keeps_state_and_records_the_error_and_envelope<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 1})).await;
        let m = t.send_payload(&a, json!({"p": 7})).await;
        let claim = t.claim_one().await;
        let committed = t.commit_ok(ending(&claim, TurnOutcome::Failed("boom".into()))).await;
        assert_eq!(committed.version, 0);
        assert_eq!(state_of(&*store, &a).await, json!({"n": 1}));
        let o = outcome_record(&*store, &m.id).await.expect("an outcome");
        assert_eq!(o.status, OutcomeStatus::Failed);
        assert_eq!(o.error.as_deref(), Some("boom"));
        assert_eq!(o.envelope.payload, json!({"p": 7}));
        assert_eq!(o.envelope.attempt, 1);
        assert_eq!(o.actor, a);
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S3 Per-instance linearizability
// ------------------------------------------------------------------------------------------------------------------

/// S3 - every read sees the preceding acknowledged write
pub async fn s3_every_read_sees_the_preceding_acknowledged_write<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"v": 0})).await;
        for i in 0..5 {
            let m = t.send(&a).await;
            assert_eq!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(m.clone())));
            assert_eq!(t.pending(&a).await, vec![m.clone()]);
            let c = t.claim_one_long().await;
            assert_eq!(c.state, json!({"v": i}));
            assert_eq!(c.version, i);
            assert_matches!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. }));
            t.commit_ok(CommitRequest { state: StateChange::NewState(json!({"v": i + 1})), ..done(&c) }).await;
            assert_eq!(state_of(&*store, &a).await, json!({"v": i + 1}));
            assert_eq!(version_of(&*store, &a).await, i + 1);
            let status = store.status(&m.id).await.unwrap();
            assert_eq!(expect_matches!(status, Some(MessageStatus::Finished(o)) => o).status, OutcomeStatus::Done);
        }
        let m = t.send(&a).await;
        assert_matches!(store.cancel(&m.id).await.unwrap(), CancelResult::Cancelled(_));
        assert_eq!(outcome_status(&*store, &m.id).await, Some(OutcomeStatus::Cancelled));
    })
    .await
}

/// S3 - concurrent enqueues to one instance get distinct seqs in one order
pub async fn s3_concurrent_enqueues_to_one_instance_get_distinct_seqs_in_one_order<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let sent: Vec<Message> = t
            .concurrently(8, |worker, peer| {
                let a = a.clone();
                async move {
                    let mut out = Vec::new();
                    for i in 0..25 {
                        let message = OutgoingMessage::new(a.clone(), "Msg", json!(format!("{worker}-{i}")));
                        out.push(enqueue_ok(&*peer, message).await);
                    }
                    out
                }
            })
            .await
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(sent.iter().map(|m| m.seq).collect::<HashSet<_>>().len(), 200);
        let pending = t.all_pending(&a, 7).await;
        assert_eq!(pending.len(), 200);
        assert_delivery_order(&pending);
        assert_eq!(
            sent.iter().map(|m| m.id.clone()).collect::<HashSet<_>>(),
            pending.iter().map(|m| m.id.clone()).collect::<HashSet<_>>()
        );
        // Per sender, its own sends keep their order.
        let mut by_sender: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for m in &pending {
            let (worker, i) = m.payload.as_str().expect("a string payload").split_once('-').expect("worker-i");
            by_sender.entry(worker.to_owned()).or_default().push(i.parse().expect("a number"));
        }
        for order in by_sender.values() {
            assert_eq!(order, &(0..25).collect::<Vec<u32>>());
        }
    })
    .await
}

/// S2 / S3 - a page of records sees each commit whole: a state with its version, and its creates with it
pub async fn s3_a_page_of_records_sees_each_commit_whole_with_its_state_version_and_creates<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let main = t.new_actor_named(&TYPE, "main", json!({"n": 0})).await;
        for round in 0..12i64 {
            t.send(&main).await;
            let c = t.claim_one().await;
            let seen = t
                .concurrently(4, |i, peer| {
                    let (c, main) = (c.clone(), main.clone());
                    async move {
                        if i == 0 {
                            let request = CommitRequest {
                                state: StateChange::NewState(json!({"n": round + 1})),
                                creates: vec![ActorCreate {
                                    address: address(&format!("conformance/main-c{round:02}")),
                                    state: json!({"round": round}),
                                    child: true,
                                }],
                                ..done(&c)
                            };
                            assert_matches!(peer.commit(request).await.unwrap(), CommitResult::Committed { .. });
                            return Vec::new();
                        }
                        let mut seen = Vec::new();
                        for _ in 0..4 {
                            let page = peer.list_records("conformance/", None, 1000).await.unwrap();
                            let row = page.items.iter().find(|r| r.address == main).expect("the main actor");
                            let n = row.state["n"].as_i64().expect("a number");
                            let children = page.items.iter().filter(|r| r.parent.as_ref() == Some(&main)).count();
                            assert_eq!(row.version, n, "a state comes with its own version");
                            assert_eq!(children as i64, n, "a commit's creates come with its state");
                            seen.push(n);
                            tokio::task::yield_now().await;
                        }
                        seen
                    }
                })
                .await;
            assert!(seen.iter().flatten().all(|n| *n == round || *n == round + 1));
        }
        let page = t.store().list_records("conformance/", None, 1000).await.unwrap();
        assert_eq!(page.items.len(), 13);
        assert_eq!(page.items[0].version, 12);
    })
    .await
}

/// S3 - concurrent creates of one instance create it once
pub async fn s3_concurrent_creates_of_one_instance_create_it_once<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = address("conformance/racy");
        t.register_type(&TYPE).await;
        let results = t
            .concurrently(10, |i, peer| {
                let a = a.clone();
                async move { peer.create(&a, json!({"by": i})).await.unwrap() }
            })
            .await;
        assert_eq!(results.iter().filter(|r| **r == CreateResult::Created).count(), 1);
        assert_eq!(results.iter().filter(|r| matches!(r, CreateResult::Exists(_))).count(), 9);
        assert!(t.store().get(&a).await.unwrap().is_some());
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S4 Lease exclusivity
// ------------------------------------------------------------------------------------------------------------------

/// S4 - a valid lease excludes other claims
pub async fn s4_a_valid_lease_excludes_other_claims<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one_long_node(&NODE_A).await;
        assert_eq!(c.actor, a);
        assert_eq!(c.message.id, m.id);
        assert_eq!(c.message.attempt, 1);
        assert!(t.claim_node(&NODE_B).await.is_empty());
        assert!(t.claim_node(&NODE_A).await.is_empty(), "even the same node cannot claim twice");
        let status = store.status(&m.id).await.unwrap();
        let (node, lease_until) =
            expect_matches!(status, Some(MessageStatus::Running { node, lease_until, .. }) => (node, lease_until));
        assert_eq!(node, *NODE_A);
        assert_eq!(lease_until, c.lease_until);
    })
    .await
}

/// S4 - concurrent claims from many nodes never share an instance
pub async fn s4_concurrent_claims_from_many_nodes_never_share_an_instance<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let mut actors = Vec::new();
        for _ in 0..30 {
            actors.push(t.new_actor(&TYPE, json!({})).await);
        }
        for a in &actors {
            t.send(a).await;
        }
        let ttl = LONG_LEASE;
        let claims: Vec<Claim> = t
            .concurrently(12, |i, peer| async move {
                let node = node(&format!("node-{i}"));
                let mut mine = Vec::new();
                for _ in 0..10 {
                    mine.extend(claim_on(&*peer, &[&TYPE], &node, ttl, 2).await);
                    tokio::task::yield_now().await;
                }
                mine
            })
            .await
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(claims.len(), 30, "every due instance claimed exactly once");
        assert_eq!(
            actors.iter().cloned().collect::<HashSet<_>>(),
            claims.iter().map(|c| c.actor.clone()).collect::<HashSet<_>>()
        );
        assert_eq!(claims.iter().map(|c| c.epoch).collect::<HashSet<_>>().len(), 30, "epochs are unique");
        assert!(claims.iter().all(|c| c.message.attempt == 1));
    })
    .await
}

/// S4 - concurrent claims of one instance yield exactly one lease
pub async fn s4_concurrent_claims_of_one_instance_yield_exactly_one_lease<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let ttl = LONG_LEASE;
        for round in 0..3 {
            let claims: Vec<Claim> = t
                .concurrently(16, |i, peer| async move {
                    claim_on(&*peer, &[&TYPE], &node(&format!("n{round}-{i}")), ttl, 10).await
                })
                .await
                .into_iter()
                .flatten()
                .collect();
            assert_eq!(claims.len(), 1, "round {round}: {claims:?}");
            t.commit_ok(done(&claims[0])).await;
            t.send(&a).await;
        }
    })
    .await
}

/// S4 - an expired lease is claimed again with a higher epoch and attempt
pub async fn s4_an_expired_lease_is_claimed_again_with_a_higher_epoch_and_attempt<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let first = t.claim_one_node(&NODE_A).await;
        t.expire_lease().await;
        assert_matches!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(_)));
        let second = t.claim_one_long_node(&NODE_B).await;
        assert_eq!(second.message.id, m.id);
        assert_eq!(second.message.attempt, 2);
        assert!(second.epoch > first.epoch);
        let status = store.status(&m.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Running { node, .. }) => node), *NODE_B);
    })
    .await
}

/// S4 - epochs increase with every claim of an instance
pub async fn s4_epochs_increase_with_every_claim_of_an_instance<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let mut epochs = Vec::new();
        for _ in 0..3 {
            t.send(&a).await;
            let c = t.claim_one().await;
            epochs.push(c.epoch);
            store.release(&a, c.epoch).await.unwrap();
            let again = t.claim_one().await;
            epochs.push(again.epoch);
            t.expire_lease().await;
            let third = t.claim_one_node(&NODE_B).await;
            epochs.push(third.epoch);
            t.commit_ok(done(&third)).await;
        }
        let mut sorted = epochs.clone();
        sorted.sort();
        assert_eq!(epochs, sorted);
        assert_eq!(epochs.iter().collect::<HashSet<_>>().len(), epochs.len());
    })
    .await
}

/// S4 - epochs stay monotonic across delete and re-create
pub async fn s4_epochs_stay_monotonic_across_delete_and_re_create<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let old = t.claim_one().await;
        assert_matches!(store.delete(&a, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. });
        assert_eq!(store.create(&a, json!({})).await.unwrap(), CreateResult::Created);
        t.send(&a).await;
        let fresh = t.claim_one().await;
        assert!(fresh.epoch > old.epoch);
        assert_eq!(t.commit(done(&old)).await, CommitResult::LeaseLost);
        assert_eq!(store.renew(&a, old.epoch, t.ttl()).await.unwrap(), RenewResult::LeaseLost);
        t.commit_ok(done(&fresh)).await;
        // Delete and re-create again, now without an outstanding lease.
        store.delete(&a, &MessageId::random()).await.unwrap();
        store.create(&a, json!({})).await.unwrap();
        t.send(&a).await;
        assert!(t.claim_one().await.epoch > fresh.epoch);
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S5 Fencing
// ------------------------------------------------------------------------------------------------------------------

/// S5 - a stale epoch cannot commit after a re-claim
pub async fn s5_a_stale_epoch_cannot_commit_after_a_re_claim<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({"n": 0})).await;
        let target = t.new_actor(&OTHER, json!({})).await;
        let m = t.send(&a).await;
        let stale = t.claim_one_node(&NODE_A).await;
        t.expire_lease().await;
        let current = t.claim_one_long_node(&NODE_B).await;
        let ghost = address("other/ghost");
        let result = t
            .commit(CommitRequest {
                state: StateChange::NewState(json!({"n": 666})),
                creates: vec![ActorCreate { address: ghost.clone(), state: json!({}), child: false }],
                sends: vec![OutgoingMessage::new(target.clone(), "T", json!(1))],
                ..done(&stale)
            })
            .await;
        assert_eq!(result, CommitResult::LeaseLost);
        assert_eq!(state_of(&*store, &a).await, json!({"n": 0}));
        assert_eq!(store.get(&ghost).await.unwrap(), None);
        assert!(t.pending(&target).await.is_empty());
        let status = store.status(&m.id).await.unwrap();
        assert_eq!(
            expect_matches!(status, Some(MessageStatus::Running { message, .. }) => message).id,
            current.message.id
        );
        t.commit_ok(CommitRequest { state: StateChange::NewState(json!({"n": 1})), ..done(&current) }).await;
        assert_eq!(state_of(&*store, &a).await, json!({"n": 1}));
    })
    .await
}

/// S5 - renew succeeds only with the current held lease
pub async fn s5_renew_succeeds_only_with_the_current_held_lease<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let ttl = t.ttl();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let c = t.claim_one().await;
        t.advance_ticks(2).await;
        let renewed =
            expect_matches!(store.renew(&a, c.epoch, LONG_LEASE).await.unwrap(), RenewResult::Renewed(until) => until);
        assert!(renewed > c.lease_until);
        t.advance_ticks(3).await; // past the original expiry, within the renewed lease
        assert!(t.claim_node(&NODE_B).await.is_empty(), "a renewed lease stays exclusive");
        assert_eq!(store.renew(&a, c.epoch + 1_000_000, ttl).await.unwrap(), RenewResult::LeaseLost);
        assert_eq!(store.renew(&address("conformance/none"), c.epoch, ttl).await.unwrap(), RenewResult::LeaseLost);
        t.commit_ok(done(&c)).await;
        assert_eq!(store.renew(&a, c.epoch, ttl).await.unwrap(), RenewResult::LeaseLost, "no renew after commit");
        t.send(&a).await;
        let c2 = t.claim_one().await;
        store.release(&a, c2.epoch).await.unwrap();
        assert_eq!(store.renew(&a, c2.epoch, ttl).await.unwrap(), RenewResult::LeaseLost, "no renew after release");
        let c3 = t.claim_one().await;
        t.expire_lease().await;
        let c4 = t.claim_one_node(&NODE_B).await;
        assert_eq!(store.renew(&a, c3.epoch, ttl).await.unwrap(), RenewResult::LeaseLost, "no renew after re-claim");
        assert_matches!(store.renew(&a, c4.epoch, ttl).await.unwrap(), RenewResult::Renewed(_));
    })
    .await
}

/// S5 - commit after release or a second commit loses the lease
pub async fn s5_commit_after_release_or_a_second_commit_loses_the_lease<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let c = t.claim_one().await;
        store.release(&a, c.epoch).await.unwrap();
        store.release(&a, c.epoch).await.unwrap(); // idempotent no-op
        assert_eq!(t.commit(done(&c)).await, CommitResult::LeaseLost);
        let again = t.claim_one().await;
        t.commit_ok(done(&again)).await;
        assert_eq!(t.commit(done(&again)).await, CommitResult::LeaseLost);
        store.release(&a, again.epoch).await.unwrap(); // no-op after commit
        assert!(t.pending(&a).await.is_empty());
    })
    .await
}

/// S5 - racing stale and current commits produce exactly one outcome
pub async fn s5_racing_stale_and_current_commits_produce_exactly_one_outcome<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        for round in 0..5 {
            let a = t.new_actor(&TYPE, json!({"n": 0})).await;
            let m = t.send(&a).await;
            let stale = t.claim_one_node(&NODE_A).await;
            t.expire_lease().await;
            let current = t.claim_one_node(&NODE_B).await;
            let results = t
                .concurrently(2, |i, peer| {
                    let c = if i == 0 { stale.clone() } else { current.clone() };
                    async move {
                        let request = CommitRequest { state: StateChange::NewState(json!({"n": i + 1})), ..done(&c) };
                        peer.commit(request).await.unwrap()
                    }
                })
                .await;
            assert_eq!(results[0], CommitResult::LeaseLost, "round {round}");
            assert_matches!(&results[1], CommitResult::Committed { .. });
            assert_eq!(state_of(&*store, &a).await, json!({"n": 2}));
            assert_eq!(outcome_status(&*store, &m.id).await, Some(OutcomeStatus::Done));
        }
    })
    .await
}

/// S5 - committing a message other than the claimed one is a programming error
pub async fn s5_committing_a_message_other_than_the_claimed_one_is_a_programming_error<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        t.send(&a).await;
        let other = t.send(&a).await;
        let c = t.claim_one().await;
        assert_matches!(
            store.commit(CommitRequest::new(a.clone(), c.epoch, other.id.clone(), TurnOutcome::Done)).await,
            Err(StoreError::Invalid(_))
        );
        t.commit_ok(done(&c)).await;
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S6 Order
// ------------------------------------------------------------------------------------------------------------------

/// S6 - only the head message is claimed, one turn at a time
pub async fn s6_only_the_head_message_is_claimed_one_turn_at_a_time<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let mut msgs = Vec::new();
        for _ in 0..5 {
            msgs.push(t.send(&a).await);
        }
        let mut handled = Vec::new();
        for _ in 0..5 {
            let claims = t.claim_with(&[&TYPE], &NODE_A, LONG_LEASE, 10).await;
            assert_eq!(claims.len(), 1, "one turn per instance even with several due messages");
            assert!(t.claim_node(&NODE_B).await.is_empty());
            handled.push(claims[0].message.id.clone());
            t.commit_ok(done(&claims[0])).await;
        }
        assert_eq!(handled, ids(&msgs));
        assert!(t.claim().await.is_empty());
    })
    .await
}

/// Claims and commits `count` turns of [`TYPE`] one by one; returns the claimed messages.
async fn handle_in_turn<F: StoreFixture>(
    t: &Env<F>,
    types: &[&crate::address::TypeName],
    count: usize,
) -> Vec<Message> {
    let mut out = Vec::new();
    for _ in 0..count {
        let c = t.claim_one_types(types).await;
        t.commit_ok(done(&c)).await;
        out.push(c.message);
    }
    out
}

/// S6 - equal deliverAt keeps enqueue order
pub async fn s6_equal_deliver_at_keeps_enqueue_order<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let a = t.new_actor(&TYPE, json!({})).await;
        let at = plus(t.store().now().await.unwrap(), t.ticks(3));
        let mut msgs = Vec::new();
        for _ in 0..6 {
            msgs.push(t.send_at(&a, Delivery::At(at)).await);
        }
        assert!(msgs.iter().all(|m| m.deliver_at == at));
        t.advance_ticks(3).await;
        assert_eq!(ids(&handle_in_turn(t, &[&TYPE], 6).await), ids(&msgs));
    })
    .await
}

/// S6 - sends from one turn with equal deliverAt keep send order
pub async fn s6_sends_from_one_turn_with_equal_deliver_at_keep_send_order<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        t.send(&a).await;
        let sends = (1..=5)
            .map(|i| OutgoingMessage {
                delivery: Delivery::After(t.tick),
                ..OutgoingMessage::new(b.clone(), format!("T{i}"), json!(i))
            })
            .collect();
        let c = t.claim_one().await;
        let committed = t.commit_ok(CommitRequest { sends, ..done(&c) }).await;
        assert_eq!(
            committed.sent.iter().map(|m| m.deliver_at).collect::<HashSet<_>>().len(),
            1,
            "one turn's sends share deliverAt"
        );
        let mut by_seq = committed.sent.clone();
        by_seq.sort_by_key(|m| m.seq);
        assert_eq!(by_seq, committed.sent);
        t.advance_ticks(2).await;
        let order: Vec<String> = handle_in_turn(t, &[&OTHER], 5).await.into_iter().map(|m| m.message_type).collect();
        assert_eq!(order, vec!["T1", "T2", "T3", "T4", "T5"]);
    })
    .await
}

/// S6 - a message is never claimed before its deliverAt
pub async fn s6_a_message_is_never_claimed_before_its_deliver_at<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send_at(&a, Delivery::After(t.ticks(10))).await;
        assert!(t.claim().await.is_empty());
        assert_eq!(t.store().next_due_at(&type_set(&[&TYPE])).await.unwrap(), Some(m.deliver_at));
        t.advance_ticks(5).await;
        assert!(t.claim().await.is_empty());
        t.advance_ticks(5).await;
        assert_eq!(t.claim_one().await.message.id, m.id);
    })
    .await
}

/// S6 - due exactly at deliverAt and not one microsecond earlier
pub async fn s6_due_exactly_at_deliver_at_and_not_one_microsecond_earlier<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        if !t.require_virtual_time() {
            return;
        }
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send_at(&a, Delivery::After(t.ticks(10))).await;
        t.advance(t.ticks(10) - Duration::from_micros(1)).await;
        assert!(t.claim().await.is_empty());
        t.advance(Duration::from_micros(1)).await;
        assert_eq!(t.store().now().await.unwrap(), m.deliver_at);
        assert_eq!(t.claim_one().await.message.id, m.id);
    })
    .await
}

/// S6 - an earlier scheduled message overtakes a later pending one
pub async fn s6_an_earlier_scheduled_message_overtakes_a_later_pending_one<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let late = t.send_at(&a, Delivery::After(t.ticks(10))).await;
        let early = t.send_at(&a, Delivery::After(t.ticks(5))).await;
        assert_eq!(t.pending(&a).await, vec![early.clone(), late.clone()]);
        t.advance_ticks(10).await;
        let c = t.claim_one().await;
        t.commit_ok(done(&c)).await;
        assert_eq!(c.message.id, early.id);
        assert_eq!(t.claim_one().await.message.id, late.id);
    })
    .await
}

/// S6 - a past deliverAt is clamped to now and does not jump the queue
pub async fn s6_a_past_deliver_at_is_clamped_to_now_and_does_not_jump_the_queue<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let first = t.send(&a).await;
        t.advance_ticks(1).await;
        let before = store.now().await.unwrap();
        let past = t.send_at(&a, Delivery::At(minus(before, t.ticks(100)))).await;
        let after = store.now().await.unwrap();
        assert!(past.deliver_at >= before && past.deliver_at <= after, "clamped to store now");
        if t.fixture.is_virtual_time() {
            assert_eq!(past.deliver_at, before);
        }
        assert_eq!(t.pending(&a).await, vec![first.clone(), past.clone()]);
        let c = t.claim_one().await;
        t.commit_ok(done(&c)).await;
        assert_eq!(c.message.id, first.id);
        assert_eq!(t.claim_one().await.message.id, past.id);
    })
    .await
}

/// S6 - due-ness follows the new head after commit, cancel and enqueue
pub async fn s6_due_ness_follows_the_new_head_after_commit_cancel_and_enqueue<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let types = type_set(&[&TYPE]);
        let a = t.new_actor(&TYPE, json!({})).await;
        let head = t.send(&a).await;
        let future = t.send_at(&a, Delivery::After(t.ticks(10))).await;
        t.commit_ok(done(&t.claim_one().await)).await;
        assert!(t.claim().await.is_empty(), "the remaining message is not due");
        assert_eq!(store.next_due_at(&types).await.unwrap(), Some(future.deliver_at));
        let now1 = t.send(&a).await;
        assert_eq!(store.next_due_at(&types).await.unwrap(), Some(now1.deliver_at));
        assert_matches!(store.cancel(&now1.id).await.unwrap(), CancelResult::Cancelled(_));
        assert!(t.claim().await.is_empty(), "cancelling the due head leaves only a future message");
        assert_eq!(store.next_due_at(&types).await.unwrap(), Some(future.deliver_at));
        t.advance_ticks(10).await;
        assert_eq!(t.claim_one().await.message.id, future.id);
        assert_eq!(outcome_status(&*store, &head.id).await, Some(OutcomeStatus::Done));
    })
    .await
}

/// S6 - pending lists the mailbox in delivery order across pages
pub async fn s6_pending_lists_the_mailbox_in_delivery_order_across_pages<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let now = t.send(&a).await;
        let in5 = t.send_at(&a, Delivery::After(t.ticks(5))).await;
        let in2 = t.send_at(&a, Delivery::After(t.ticks(2))).await;
        let in2b = t.send_at(&a, Delivery::At(in2.deliver_at)).await;
        let now2 = t.send(&a).await;
        let listed = t.all_pending(&a, 2).await;
        assert_eq!(listed.len(), 5);
        assert_delivery_order(&listed);
        assert_eq!(ids(&listed[2..]), vec![in2.id, in2b.id, in5.id]);
        assert_eq!(ids(&listed[..2]).into_iter().collect::<HashSet<_>>(), HashSet::from([now.id, now2.id]));
        assert!(t.pending(&address("conformance/none")).await.is_empty());
    })
    .await
}

async fn list_all_pending(store: &dyn Backend, prefix: &str, page_size: usize) -> Vec<MessageId> {
    let mut out = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let page = store.list_pending(prefix, cursor.as_ref(), page_size).await.unwrap();
        assert!(page.items.len() <= page_size);
        out.extend(page.items.into_iter().map(|m| m.id));
        match page.next {
            Some(next) => cursor = Some(next),
            None => return out,
        }
    }
}

/// S6 - pending of all instances by prefix, instance by instance in delivery order, across pages
pub async fn s6_pending_of_all_instances_by_prefix_instance_by_instance_in_delivery_order_across_pages<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let b = t.new_actor_named(&TYPE, "b", json!({})).await;
        let a = t.new_actor_named(&TYPE, "a", json!({})).await;
        let other = t.new_actor_named(&OTHER, "o", json!({})).await;
        let b_now = t.send(&b).await;
        let a_later = t.send_at(&a, Delivery::After(t.ticks(3))).await;
        let a_now = t.send(&a).await;
        let a_soon = t.send_at(&a, Delivery::After(t.tick)).await;
        let other_now = t.send(&other).await;
        t.new_actor_named(&TYPE, "empty", json!({})).await;

        let expected = vec![a_now.id.clone(), a_soon.id.clone(), a_later.id.clone(), b_now.id.clone()];
        for page_size in [1, 2, 3, 100] {
            assert_eq!(list_all_pending(&*store, "conformance/", page_size).await, expected, "page size {page_size}");
        }
        assert_eq!(list_all_pending(&*store, "other/", 2).await, vec![other_now.id.clone()]);
        let mut everything = expected.clone();
        everything.push(other_now.id.clone());
        assert_eq!(list_all_pending(&*store, "", 2).await, everything);
        assert!(store.list_pending("nothing/", None, PAGE).await.unwrap().items.is_empty());

        // Messages being handled are still pending; ones that end leave the listing.
        let claimed = t.claim().await;
        assert_eq!(
            claimed.iter().map(|c| c.message.id.clone()).collect::<HashSet<_>>(),
            HashSet::from([a_now.id.clone(), b_now.id.clone()])
        );
        assert_eq!(list_all_pending(&*store, "", 2).await.len(), 5);
        for c in &claimed {
            t.commit_ok(done(c)).await;
        }
        assert_eq!(list_all_pending(&*store, "", 2).await, vec![a_soon.id, a_later.id, other_now.id]);
        assert_matches!(
            store.list_pending("", Some(&PageCursor("nonsense".into())), 2).await,
            Err(StoreError::Invalid(_))
        );
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S7 No loss, no duplicate
// ------------------------------------------------------------------------------------------------------------------

fn with_id(to: &ActorAddress, id: &MessageId) -> OutgoingMessage {
    OutgoingMessage { id: id.clone(), ..OutgoingMessage::new(to.clone(), "T", json!(1)) }
}

/// S7 - message ids are unique across pending messages and retained outcomes
pub async fn s7_message_ids_are_unique_across_pending_messages_and_retained_outcomes<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&TYPE, json!({})).await;
        let pending = t.send(&a).await;
        assert_eq!(
            store.enqueue(with_id(&a, &pending.id)).await.unwrap(),
            EnqueueResult::Duplicate(pending.id.clone())
        );
        assert_eq!(
            store.enqueue(with_id(&b, &pending.id)).await.unwrap(),
            EnqueueResult::Duplicate(pending.id.clone())
        );
        t.commit_ok(done(&t.claim_one().await)).await;
        assert_eq!(
            store.enqueue(with_id(&b, &pending.id)).await.unwrap(),
            EnqueueResult::Duplicate(pending.id.clone())
        );
        let cancelled = t.send(&b).await;
        store.cancel(&cancelled.id).await.unwrap();
        assert_eq!(
            store.enqueue(with_id(&b, &cancelled.id)).await.unwrap(),
            EnqueueResult::Duplicate(cancelled.id.clone())
        );
        let purged = t.send(&b).await;
        store.delete(&b, &MessageId::random()).await.unwrap();
        assert_eq!(store.enqueue(with_id(&a, &purged.id)).await.unwrap(), EnqueueResult::Duplicate(purged.id.clone()));
        assert_eq!(outcome_status(&*store, &pending.id).await, Some(OutcomeStatus::Done));
        assert!(t.pending(&a).await.is_empty());
    })
    .await
}

/// S7 - concurrent enqueues of one id accept exactly one
pub async fn s7_concurrent_enqueues_of_one_id_accept_exactly_one<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        for _ in 0..3 {
            let id = MessageId::random();
            let results = t
                .concurrently(12, |i, peer| {
                    let message = OutgoingMessage { id: id.clone(), ..OutgoingMessage::new(a.clone(), "T", json!(i)) };
                    async move { peer.enqueue(message).await.unwrap() }
                })
                .await;
            assert_eq!(results.iter().filter(|r| matches!(r, EnqueueResult::Enqueued(_))).count(), 1, "{results:?}");
            assert_eq!(results.iter().filter(|r| **r == EnqueueResult::Duplicate(id.clone())).count(), 11);
        }
        assert_eq!(t.pending(&a).await.len(), 3);
    })
    .await
}

/// S7 - a send to an absent instance is refused and does not consume the id
pub async fn s7_a_send_to_an_absent_instance_is_refused_and_does_not_consume_the_id<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = address("conformance/later");
        t.register_type(&TYPE).await;
        let id = MessageId::random();
        assert_eq!(store.enqueue(with_id(&a, &id)).await.unwrap(), EnqueueResult::NoSuchInstance);
        assert_eq!(store.status(&id).await.unwrap(), None);
        store.create(&a, json!({})).await.unwrap();
        assert_matches!(store.enqueue(with_id(&a, &id)).await.unwrap(), EnqueueResult::Enqueued(_));
    })
    .await
}

/// S7 - cancel ends only pending messages
pub async fn s7_cancel_ends_only_pending_messages<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let running = t.send(&a).await;
        let waiting = t.send(&a).await;
        let claim = t.claim_one_long().await;
        assert_eq!(store.cancel(&running.id).await.unwrap(), CancelResult::Running);
        let cancelled = expect_matches!(store.cancel(&waiting.id).await.unwrap(), CancelResult::Cancelled(o) => o);
        assert_eq!(cancelled.status, OutcomeStatus::Cancelled);
        assert_eq!(cancelled.envelope.id, waiting.id);
        assert_eq!(outcome_status(&*store, &waiting.id).await, Some(OutcomeStatus::Cancelled));
        let finished = expect_matches!(store.cancel(&waiting.id).await.unwrap(), CancelResult::AlreadyFinished(o) => o);
        assert_eq!(finished.status, OutcomeStatus::Cancelled);
        t.commit_ok(done(&claim)).await;
        let finished = expect_matches!(store.cancel(&running.id).await.unwrap(), CancelResult::AlreadyFinished(o) => o);
        assert_eq!(finished.status, OutcomeStatus::Done);
        assert_eq!(store.cancel(&MessageId::random()).await.unwrap(), CancelResult::NotFound);
        assert!(t.pending(&a).await.is_empty());
    })
    .await
}

/// S7 - cancelling a claimed message whose lease expired fences the stale turn
pub async fn s7_cancelling_a_claimed_message_whose_lease_expired_fences_the_stale_turn<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let claim = t.claim_one().await;
        t.expire_lease().await;
        assert_matches!(store.cancel(&m.id).await.unwrap(), CancelResult::Cancelled(_));
        assert_eq!(t.commit(done(&claim)).await, CommitResult::LeaseLost);
        assert_eq!(outcome_status(&*store, &m.id).await, Some(OutcomeStatus::Cancelled));
        let next = t.send(&a).await;
        assert_eq!(t.claim_one_node(&NODE_B).await.message.id, next.id);
    })
    .await
}

/// S7 - release, expiry and lost commits keep the message pending
pub async fn s7_release_expiry_and_lost_commits_keep_the_message_pending<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c1 = t.claim_one().await;
        store.release(&a, c1.epoch).await.unwrap();
        let status = store.status(&m.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Pending(p)) => p).attempt, 1);
        let c2 = t.claim_one().await;
        t.expire_lease().await;
        let status = store.status(&m.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Pending(p)) => p).attempt, 2);
        let c3 = t.claim_one_long_node(&NODE_B).await;
        assert_eq!(t.commit(done(&c2)).await, CommitResult::LeaseLost);
        let status = store.status(&m.id).await.unwrap();
        assert_eq!(expect_matches!(status, Some(MessageStatus::Running { message, .. }) => message).attempt, 3);
        assert_eq!(outcome_record(&*store, &m.id).await, None);
        t.commit_ok(done(&c3)).await;
        assert_eq!(outcome_record(&*store, &m.id).await.expect("an outcome").envelope.attempt, 3);
    })
    .await
}

/// What the workers of the serial-processing case observed.
#[derive(Default)]
struct Workers {
    in_flight: Mutex<HashSet<ActorAddress>>,
    /// Claimed messages whose commit has not returned yet: covers the commit itself. A second claim of a message
    /// before its commit returned means two turns overlapped (the next legitimate claim gets the next message).
    uncommitted: Mutex<HashSet<MessageId>>,
    processed: Mutex<Vec<(ActorAddress, i64)>>,
    violations: AtomicUsize,
    remaining: AtomicI64,
}

/// S7 - concurrent workers process every message exactly once, serially and in order
pub async fn s7_concurrent_workers_process_every_message_exactly_once_serially_and_in_order<B: FixtureFactory>(
    backend: &B,
) {
    store_test_with(backend, backend.long_retention(), Duration::from_secs(180), async |t| {
        let store = t.store();
        let mut actors = Vec::new();
        for _ in 0..6 {
            actors.push(t.new_actor(&TYPE, json!({})).await);
        }
        let per_actor = 15;
        // Concurrent producers.
        let actors = Arc::new(actors);
        let sent: Vec<Vec<Message>> = t
            .concurrently(actors.len(), |i, peer| {
                let actors = actors.clone();
                async move {
                    let mut out = Vec::new();
                    for k in 0..per_actor {
                        out.push(enqueue_ok(&*peer, OutgoingMessage::new(actors[i].clone(), "Msg", json!(k))).await);
                    }
                    out
                }
            })
            .await;
        let expected: HashSet<MessageId> = sent.iter().flatten().map(|m| m.id.clone()).collect();
        let workers = Arc::new(Workers { remaining: AtomicI64::new(expected.len() as i64), ..Workers::default() });
        let ttl = t.ttl() * 10;
        t.concurrently(6, |w, peer| {
            let workers = workers.clone();
            async move {
                let node = node(&format!("worker-{w}"));
                while workers.remaining.load(Ordering::SeqCst) > 0 {
                    let claims = claim_on(&*peer, &[&TYPE], &node, ttl, 2).await;
                    if claims.is_empty() {
                        tokio::task::yield_now().await;
                        continue;
                    }
                    for c in claims {
                        if !workers.in_flight.lock().unwrap().insert(c.actor.clone()) {
                            workers.violations.fetch_add(1, Ordering::SeqCst);
                        }
                        if !workers.uncommitted.lock().unwrap().insert(c.message.id.clone()) || c.message.attempt != 1 {
                            workers.violations.fetch_add(1, Ordering::SeqCst);
                        }
                        workers.processed.lock().unwrap().push((c.actor.clone(), c.message.seq));
                        tokio::task::yield_now().await;
                        workers.in_flight.lock().unwrap().remove(&c.actor);
                        let request = CommitRequest { state: StateChange::NewState(json!(c.message.seq)), ..done(&c) };
                        assert_matches!(peer.commit(request).await.unwrap(), CommitResult::Committed { .. });
                        workers.uncommitted.lock().unwrap().remove(&c.message.id);
                        workers.remaining.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            }
        })
        .await;
        assert_eq!(workers.violations.load(Ordering::SeqCst), 0, "turns of one instance overlapped");
        let processed = workers.processed.lock().unwrap().clone();
        assert_eq!(processed.len(), expected.len(), "no message handled twice");
        for id in &expected {
            assert_eq!(outcome_status(&*store, id).await, Some(OutcomeStatus::Done), "outcome of {id}");
        }
        for (i, a) in actors.iter().enumerate() {
            let seqs: Vec<i64> = processed.iter().filter(|p| &p.0 == a).map(|p| p.1).collect();
            assert_eq!(seqs, sent[i].iter().map(|m| m.seq).collect::<Vec<_>>(), "per-instance order for {a}");
            assert!(t.pending(a).await.is_empty());
            assert_eq!(version_of(&*store, a).await, per_actor as i64);
        }
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S8 Liveness
// ------------------------------------------------------------------------------------------------------------------

/// S8 - a released lease is claimable again at once
pub async fn s8_a_released_lease_is_claimable_again_at_once<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        let c = t.claim_one_node(&NODE_A).await;
        t.store().release(&a, c.epoch).await.unwrap();
        let again = t.claim_one_node(&NODE_B).await;
        assert_eq!(again.message.id, m.id);
        assert_eq!(again.message.attempt, 2);
    })
    .await
}

/// S8 - every due instance is eventually claimed under small limits
pub async fn s8_every_due_instance_is_eventually_claimed_under_small_limits<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let mut actors = HashSet::new();
        for _ in 0..17 {
            actors.insert(t.new_actor(&TYPE, json!({})).await);
        }
        for a in &actors {
            t.send(a).await;
        }
        let mut claimed = HashSet::new();
        for _ in 0..20 {
            let batch = t.claim_limit(3).await;
            assert!(batch.len() <= 3);
            for c in batch {
                claimed.insert(c.actor.clone());
                t.commit_ok(done(&c)).await;
            }
        }
        assert_eq!(claimed, actors);
        assert!(t.claim_limit(0).await.is_empty());
    })
    .await
}

/// S8 - claimDue only returns the requested types
pub async fn s8_claim_due_only_returns_the_requested_types<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        t.send(&a).await;
        t.send(&b).await;
        assert_eq!(t.claim_one_types(&[&OTHER]).await.actor, b);
        assert!(t.claim_types(&[&OTHER]).await.is_empty());
        assert!(t.claim_types(&[]).await.is_empty());
        assert_eq!(t.claim_one_types(&[&TYPE, &OTHER]).await.actor, a);
    })
    .await
}

/// S8 - nextDueAt reports when an instance becomes claimable
pub async fn s8_next_due_at_reports_when_an_instance_becomes_claimable<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let types = type_set(&[&TYPE]);
        assert_eq!(store.next_due_at(&types).await.unwrap(), None);
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let later = t.send_at(&a, Delivery::After(t.ticks(20))).await;
        t.send_at(&b, Delivery::After(t.ticks(2))).await;
        assert_eq!(store.next_due_at(&types).await.unwrap(), Some(later.deliver_at));
        let sooner = t.send_at(&a, Delivery::After(t.ticks(5))).await;
        assert_eq!(store.next_due_at(&types).await.unwrap(), Some(sooner.deliver_at));
        assert!(store.next_due_at(&type_set(&[&OTHER])).await.unwrap().expect("a due time") < sooner.deliver_at);
        assert_eq!(store.next_due_at(&BTreeSet::new()).await.unwrap(), None);
        let c = t.new_actor(&TYPE, json!({})).await;
        t.send(&c).await;
        let claim = t.claim_one().await;
        // A leased instance becomes claimable again no earlier than its lease expiry.
        assert_eq!(store.next_due_at(&types).await.unwrap(), Some(claim.lease_until.min(sooner.deliver_at)));
        t.commit_ok(done(&claim)).await;
        assert_eq!(store.next_due_at(&types).await.unwrap(), Some(sooner.deliver_at));
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S9 One clock
// ------------------------------------------------------------------------------------------------------------------

/// S9 - due-ness, delays and leases are judged by the store clock
pub async fn s9_due_ness_delays_and_leases_are_judged_by_the_store_clock<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let t0 = store.now().await.unwrap();
        let m = t.send_at(&a, Delivery::After(t.ticks(3))).await;
        let t1 = store.now().await.unwrap();
        assert!(m.deliver_at >= plus(t0, t.ticks(3)) && m.deliver_at <= plus(t1, t.ticks(3)));
        t.advance_ticks(3).await;
        let before = store.now().await.unwrap();
        let c = t.claim_one_ttl(t.ticks(2)).await;
        let after = store.now().await.unwrap();
        assert!(c.claimed_at >= before && c.claimed_at <= after);
        assert_eq!(c.lease_until, plus(c.claimed_at, t.ticks(2)));
        assert!(c.claimed_at >= m.deliver_at);
        if t.fixture.is_virtual_time() {
            assert_eq!(t0, t1);
            assert_eq!(m.deliver_at, plus(t0, t.ticks(3)));
            assert_eq!(c.claimed_at, before);
        }
    })
    .await
}

/// S9 - store time never goes backwards
pub async fn s9_store_time_never_goes_backwards<B: FixtureFactory>(backend: &B) {
    store_test(backend, async |t| {
        let store = t.store();
        let mut last = store.now().await.unwrap();
        for _ in 0..20 {
            let n = store.now().await.unwrap();
            assert!(n >= last);
            last = n;
        }
    })
    .await
}

// ------------------------------------------------------------------------------------------------------------------
// S10 Retention
// ------------------------------------------------------------------------------------------------------------------

/// S10 - outcomes are retained for the retention period, then removed
pub async fn s10_outcomes_are_retained_for_the_retention_period_then_removed<B: FixtureFactory>(backend: &B) {
    store_test_with(backend, tick_retention(backend), DEFAULT_TIMEOUT, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        t.commit_ok(done(&t.claim_one().await)).await;
        let cancelled = t.send(&a).await;
        store.cancel(&cancelled.id).await.unwrap();
        assert!(outcome_record(&*store, &m.id).await.is_some());
        t.advance_ticks(RETENTION_TICKS / 2).await;
        assert!(outcome_record(&*store, &m.id).await.is_some());
        assert_matches!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Finished(_)));
        assert_eq!(store.enqueue(with_id(&a, &m.id)).await.unwrap(), EnqueueResult::Duplicate(m.id.clone()));
        t.advance_ticks(RETENTION_TICKS / 2 + 1).await;
        assert_eq!(outcome_record(&*store, &m.id).await, None, "expired outcomes are invisible before a sweep");
        assert_eq!(store.status(&m.id).await.unwrap(), None);
        assert_eq!(store.cancel(&cancelled.id).await.unwrap(), CancelResult::NotFound);
        assert_eq!(store.sweep_expired().await.unwrap(), 2, "both expired outcomes are swept");
        assert_eq!(store.sweep_expired().await.unwrap(), 0);
    })
    .await
}

/// S10 - a message id can be reused once its outcome expired
pub async fn s10_a_message_id_can_be_reused_once_its_outcome_expired<B: FixtureFactory>(backend: &B) {
    store_test_with(backend, tick_retention(backend), DEFAULT_TIMEOUT, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let m = t.send(&a).await;
        t.commit_ok(done(&t.claim_one().await)).await;
        t.advance_ticks(RETENTION_TICKS + 1).await;
        // Without a sweep first: the expired outcome no longer blocks the id.
        let again = t.send_id(&a, m.id.clone()).await;
        assert_eq!(again.id, m.id);
        assert_matches!(store.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(_)));
        t.commit_ok(done(&t.claim_one().await)).await;
        assert_eq!(outcome_status(&*store, &m.id).await, Some(OutcomeStatus::Done));
        store.sweep_expired().await.unwrap();
        assert!(outcome_record(&*store, &m.id).await.is_some(), "a sweep only removes expired outcomes");
    })
    .await
}

/// All outcomes of `status` and `prefix` since `since`, following pages of 2; asserts the `(finished_at, message_id)`
/// order.
async fn collect_outcomes(
    store: &dyn Backend,
    status: Option<OutcomeStatus>,
    prefix: &str,
    since: Timestamp,
) -> Vec<OutcomeRecord> {
    let mut out: Vec<OutcomeRecord> = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let page = store.outcomes(status, prefix, since, cursor.as_ref(), 2).await.unwrap();
        out.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let mut sorted = out.clone();
    sorted.sort_by(|a, b| (a.finished_at, a.message_id.as_str()).cmp(&(b.finished_at, b.message_id.as_str())));
    assert_eq!(sorted, out, "outcomes are ordered by (finishedAt, messageId)");
    out
}

async fn collect_failures(store: &dyn Backend, prefix: &str, since: Timestamp) -> Vec<OutcomeRecord> {
    collect_outcomes(store, Some(OutcomeStatus::Failed), prefix, since).await
}

/// S10 - failures are listed by prefix and since, with paging, within retention
pub async fn s10_failures_are_listed_by_prefix_and_since_with_paging_within_retention<B: FixtureFactory>(backend: &B) {
    store_test_with(backend, tick_retention(backend), DEFAULT_TIMEOUT, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let start = store.now().await.unwrap();
        let mut failed_a = Vec::new();
        for _ in 0..4 {
            let m = t.send(&a).await;
            t.commit_ok(ending(&t.claim_one().await, TurnOutcome::Failed("err-a".into()))).await;
            failed_a.push(m);
        }
        t.send(&a).await;
        t.commit_ok(done(&t.claim_one().await)).await; // done, not a failure
        t.send(&b).await;
        t.commit_ok(ending(&t.claim_one_types(&[&OTHER]).await, TurnOutcome::Failed("err-b".into()))).await;
        t.advance_ticks(2).await;
        let mid = store.now().await.unwrap();
        let late_a = t.send(&a).await;
        t.commit_ok(ending(&t.claim_one().await, TurnOutcome::Failed("late".into()))).await;

        let all = collect_failures(&*store, "conformance/", start).await;
        assert_eq!(all.len(), 5);
        assert_eq!(
            all[..4].iter().map(|o| o.message_id.clone()).collect::<HashSet<_>>(),
            failed_a.iter().map(|m| m.id.clone()).collect::<HashSet<_>>()
        );
        let last = all.last().expect("a failure");
        assert_eq!(last.message_id, late_a.id);
        assert!(all.iter().all(|o| o.status == OutcomeStatus::Failed && o.actor == a));
        assert_eq!(last.error.as_deref(), Some("late"));
        assert_eq!(last.envelope.attempt, 1);
        let since_mid: Vec<MessageId> =
            collect_failures(&*store, "conformance/", mid).await.into_iter().map(|o| o.message_id).collect();
        assert_eq!(since_mid, vec![late_a.id.clone()]);
        let other: Vec<Option<String>> =
            collect_failures(&*store, "other/", start).await.into_iter().map(|o| o.error).collect();
        assert_eq!(other, vec![Some("err-b".to_owned())]);
        assert_eq!(collect_failures(&*store, "", start).await.len(), 6);
        t.advance_ticks(RETENTION_TICKS + 1).await;
        assert!(collect_failures(&*store, "", start).await.is_empty());
    })
    .await
}

/// stats - actors and pending messages are counted under a prefix, and the oldest due message is found
pub async fn stats_count_actors_and_pending_messages_under_a_prefix_and_find_the_oldest_due_one<B: FixtureFactory>(
    backend: &B,
) {
    store_test(backend, async |t| {
        let store = t.store();
        let none = StoreStats { actors: 0, pending: 0, oldest_due: None };
        assert_eq!(store.stats("").await.unwrap(), none);
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        assert_eq!(store.stats("").await.unwrap(), StoreStats { actors: 2, ..none });
        let first = t.send(&a).await;
        let second = t.send(&a).await;
        let later = t.send_at(&b, Delivery::After(t.ticks(10))).await;
        let oldest = Some(first.deliver_at);
        assert_eq!(store.stats("").await.unwrap(), StoreStats { actors: 2, pending: 3, oldest_due: oldest });
        assert_eq!(
            store.stats("conformance/").await.unwrap(),
            StoreStats { actors: 1, pending: 2, oldest_due: oldest }
        );
        assert_eq!(store.stats(a.as_str()).await.unwrap(), StoreStats { actors: 1, pending: 2, oldest_due: oldest });
        assert_eq!(store.stats("other/").await.unwrap(), StoreStats { actors: 1, pending: 1, oldest_due: None });
        assert_eq!(store.stats("nothing/").await.unwrap(), none);

        // A message being handled is still pending; a finished one is not, and the next one is the oldest.
        let claim = t.claim_one().await;
        assert_eq!(store.stats("conformance/").await.unwrap().pending, 2);
        t.commit_ok(done(&claim)).await;
        assert_eq!(
            store.stats("conformance/").await.unwrap(),
            StoreStats { actors: 1, pending: 1, oldest_due: Some(second.deliver_at) }
        );

        // A delayed message counts as due once its time came.
        t.advance(t.ticks(10)).await;
        assert_eq!(
            store.stats("other/").await.unwrap(),
            StoreStats { actors: 1, pending: 1, oldest_due: Some(later.deliver_at) }
        );
        assert_eq!(store.delete(&a, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { purged: 1 });
        assert_eq!(
            store.stats("").await.unwrap(),
            StoreStats { actors: 1, pending: 1, oldest_due: Some(later.deliver_at) }
        );
    })
    .await
}

/// S10 - outcomes are listed by status, prefix and since, with paging; a hidden record stays out
pub async fn s10_outcomes_are_listed_by_status_prefix_and_since_with_paging_and_a_hidden_record_stays_out<
    B: FixtureFactory,
>(
    backend: &B,
) {
    store_test_with(backend, tick_retention(backend), DEFAULT_TIMEOUT, async |t| {
        let store = t.store();
        let a = t.new_actor(&TYPE, json!({})).await;
        let b = t.new_actor(&OTHER, json!({})).await;
        let start = store.now().await.unwrap();
        let mut done_ids = Vec::new();
        for _ in 0..3 {
            let m = t.send(&a).await;
            t.commit_ok(done(&t.claim_one().await)).await;
            done_ids.push(m.id);
        }
        let failed = t.send(&a).await;
        t.commit_ok(ending(&t.claim_one().await, TurnOutcome::Failed("err".into()))).await;
        let cancelled = t.send_at(&a, Delivery::After(t.ticks(100))).await;
        assert!(matches!(store.cancel(&cancelled.id).await.unwrap(), CancelResult::Cancelled(_)));
        let purged = t.send_at(&b, Delivery::After(t.ticks(100))).await;
        assert!(matches!(store.delete(&b, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { purged: 1 }));
        // A record removed at once is hidden from every listing.
        let hidden = enqueue_ok(
            &*store,
            OutgoingMessage {
                retention: Some(Retention::REMOVE_ON_DONE),
                ..OutgoingMessage::new(a.clone(), "Msg", json!({}))
            },
        )
        .await;
        t.commit_ok(done(&t.claim_one().await)).await;
        assert_eq!(store.status(&hidden.id).await.unwrap(), None);

        let ids = |records: Vec<OutcomeRecord>| records.into_iter().map(|o| o.message_id).collect::<HashSet<_>>();
        let status_of = |status| collect_outcomes(&*store, status, "", start);
        assert_eq!(ids(status_of(Some(OutcomeStatus::Done)).await), done_ids.iter().cloned().collect());
        assert_eq!(ids(status_of(Some(OutcomeStatus::Failed)).await), HashSet::from([failed.id.clone()]));
        assert_eq!(ids(status_of(Some(OutcomeStatus::Cancelled)).await), HashSet::from([cancelled.id.clone()]));
        assert_eq!(ids(status_of(Some(OutcomeStatus::Purged)).await), HashSet::from([purged.id.clone()]));
        let everything = status_of(None).await;
        assert_eq!(everything.len(), 6, "{everything:?}");
        assert!(everything.iter().all(|o| o.message_id != hidden.id));
        assert_eq!(
            collect_outcomes(&*store, None, "conformance/", start).await.len(),
            5,
            "the prefix limits the instances"
        );
        let after_all = store.now().await.unwrap() + chrono::TimeDelta::microseconds(1);
        assert!(collect_outcomes(&*store, None, "", after_all).await.is_empty(), "since");

        t.advance_ticks(RETENTION_TICKS + 1).await;
        assert!(collect_outcomes(&*store, None, "", start).await.is_empty());
    })
    .await
}
