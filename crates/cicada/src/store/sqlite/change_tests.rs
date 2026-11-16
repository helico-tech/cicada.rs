//! How a SQLite write transaction ends for the change feed (ADR 0034): a confirmed commit publishes its hints once, a
//! known rollback nothing, and an uncertain ending one resync after the transaction settled, before the next writer.
//! The faults stop a transaction at a known point of its ending, where the test cancels its caller.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::json;
use tokio::sync::Notify;

use super::{CommitFault, SqliteStore};
use crate::address::{ActorAddress, TypeName};
use crate::change::{ActorChange, ActorChanges};
use crate::store::{Backend, CreateResult, SqliteOptions, Store, StoreError};
use crate::tests::conformance::type_def;
use crate::tests::sqlite::common::TempDb;

fn kind() -> TypeName {
    TypeName::parse("x").unwrap()
}

fn instance(id: &str) -> ActorAddress {
    ActorAddress::of(&kind(), id).unwrap()
}

async fn open(db: &TempDb) -> Arc<SqliteStore> {
    let store = Arc::new(SqliteStore::open(&db.file, SqliteOptions::default()).await.unwrap());
    store.register(type_def(&kind(), "h")).await.unwrap();
    store
}

/// A live stream of every address: its first `Resync` is taken.
async fn subscribe(store: &dyn Backend) -> ActorChanges {
    let mut changes = ActorChanges::new(store.changes(), "");
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync));
    changes
}

async fn next(changes: &mut ActorChanges) -> Option<ActorChange> {
    tokio::time::timeout(Duration::from_secs(10), changes.next()).await.expect("an item in time")
}

/// The items up to the hint of `marker`, which the test writes last.
async fn until(changes: &mut ActorChanges, marker: &ActorAddress) -> Vec<ActorChange> {
    let mut seen = Vec::new();
    loop {
        match next(changes).await {
            Some(ActorChange::Changed { address, .. }) if &address == marker => return seen,
            Some(change) => seen.push(change),
            None => panic!("the stream ended after {seen:?}"),
        }
    }
}

async fn create(store: &dyn Backend, id: &str) {
    assert_eq!(store.create(&instance(id), json!({}), None).await.unwrap(), CreateResult::Created);
}

/// Starts a create of `id` that stops at `fault`, and returns it once it got there.
async fn stopped_create(store: &Arc<SqliteStore>, id: &str, fault: impl FnOnce(Arc<Notify>) -> CommitFault) {
    let reached = Arc::new(Notify::new());
    *store.commit_fault.lock().unwrap() = Some(fault(reached.clone()));
    let task = {
        let store = store.clone();
        let address = instance(id);
        tokio::spawn(async move { store.create(&address, json!({}), None).await })
    };
    reached.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn a_confirmed_commit_publishes_its_hints_once() {
    let db = TempDb::new();
    let store = open(&db).await;
    let mut changes = subscribe(&*store).await;
    create(&*store, "a").await;
    create(&*store, "marker").await;
    assert_eq!(until(&mut changes, &instance("marker")).await, vec![ActorChange::changed(instance("a"))]);
    store.close().await;
}

#[tokio::test]
async fn a_cancel_before_commit_is_asked_for_rolls_back_and_publishes_nothing() {
    let db = TempDb::new();
    let store = open(&db).await;
    let mut changes = subscribe(&*store).await;
    stopped_create(&store, "a", CommitFault::BeforeCommit).await;
    create(&*store, "marker").await; // the next BEGIN rolls the open transaction back
    assert_eq!(until(&mut changes, &instance("marker")).await, vec![]);
    assert_eq!(store.get(&instance("a")).await.unwrap(), None);
    store.close().await;
}

#[tokio::test]
async fn a_cancel_while_commit_is_queued_settles_then_resyncs_and_the_create_did_not_land() {
    let db = TempDb::new();
    let store = open(&db).await;
    let mut changes = subscribe(&*store).await;
    stopped_create(&store, "a", CommitFault::Queued).await;
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync));
    assert_eq!(store.get(&instance("a")).await.unwrap(), None);
    create(&*store, "marker").await;
    assert_eq!(until(&mut changes, &instance("marker")).await, vec![], "one resync, no hint of the create");
    store.close().await;
}

#[tokio::test]
async fn a_cancel_during_an_in_flight_commit_resyncs_after_it_landed_and_before_the_next_writer() {
    let db = TempDb::new();
    let store = open(&db).await;
    let mut changes = subscribe(&*store).await;
    stopped_create(&store, "a", CommitFault::InFlight).await;
    // The next writer waits for the settle task, which holds the writer until it published the resync.
    let next_writer = {
        let store = store.clone();
        tokio::spawn(async move { create(&*store, "b").await })
    };
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync));
    assert!(store.get(&instance("a")).await.unwrap().is_some(), "the commit sqlx ran after its caller left");
    next_writer.await.unwrap();
    assert_eq!(next(&mut changes).await, Some(ActorChange::changed(instance("b"))));
    store.close().await;
}

#[tokio::test]
async fn a_failed_commit_rolls_back_and_resyncs_and_a_known_rollback_publishes_nothing() {
    let db = TempDb::new();
    let store = open(&db).await;
    let mut changes = subscribe(&*store).await;
    *store.commit_fault.lock().unwrap() = Some(CommitFault::Fail);
    let failed = store.create(&instance("a"), json!({}), None).await;
    assert!(matches!(failed, Err(StoreError::Unavailable(_))), "{failed:?}");
    assert_eq!(next(&mut changes).await, Some(ActorChange::Resync));
    assert_eq!(store.get(&instance("a")).await.unwrap(), None);

    // A transaction that ends in an error rolls back: its collected hints go nowhere.
    let mut tx = store.write().await.unwrap();
    let (_, collected) = tx.parts();
    collected.add(&instance("never"));
    let ended: Result<(), _> = tx.end(Err(StoreError::Invalid("refused".into()))).await;
    assert!(ended.is_err());
    // A commit without hints (a failed one too) publishes nothing.
    *store.commit_fault.lock().unwrap() = Some(CommitFault::Fail);
    assert!(store.purge(&instance("absent")).await.is_err());
    create(&*store, "marker").await;
    assert_eq!(until(&mut changes, &instance("marker")).await, vec![]);
    store.close().await;
}

/// A runtime shut down while a commit settles: the task that would settle it never runs and no timely hint is
/// promised, but the store, used from another runtime, gets its writer back and the next `BEGIN` rolls the transaction
/// back; a fresh subscription and a full read then show the settled file.
#[test]
fn a_runtime_shut_down_during_settlement_frees_the_writer_and_a_fresh_subscription_reads_the_settled_file() {
    let main = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let db = TempDb::new();
    let store = main.block_on(open(&db));
    for (key, fault, lands) in [
        ("in-flight", CommitFault::InFlight as fn(Arc<Notify>) -> CommitFault, true),
        ("queued", CommitFault::Queued, false),
    ] {
        let doomed = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        doomed.block_on(async {
            let reached = Arc::new(Notify::new());
            *store.commit_fault.lock().unwrap() = Some(fault(reached.clone()));
            let store = store.clone();
            tokio::spawn(async move { store.create(&instance(key), json!({}), None).await });
            reached.notified().await;
        });
        drop(doomed); // drops the stopped create, and with it the settle task it asks for
        main.block_on(async {
            let after = format!("after-{key}");
            let next_write = tokio::time::timeout(Duration::from_secs(10), create(&*store, &after));
            next_write.await.expect("the writer is free again");
            let mut fresh = subscribe(&*store).await;
            assert_eq!(store.get(&instance(key)).await.unwrap().is_some(), lands, "{key}");
            create(&*store, &format!("marker-{key}")).await;
            assert_eq!(until(&mut fresh, &instance(&format!("marker-{key}"))).await, vec![]);
        });
    }
    main.block_on(store.close());
}

#[tokio::test]
async fn close_waits_for_a_settling_commit_and_ends_the_streams() {
    let db = TempDb::new();
    let store = open(&db).await;
    let mut changes = subscribe(&*store).await;
    stopped_create(&store, "a", CommitFault::InFlight).await;
    store.close().await;
    assert_eq!(next(&mut changes).await, None);
    assert!(store.get(&instance("a")).await.unwrap_err().is_unavailable());
    let reopened = open(&db).await;
    assert!(reopened.get(&instance("a")).await.unwrap().is_some(), "the settled commit is in the file");
    reopened.close().await;
}

#[tokio::test]
async fn clones_of_a_store_share_its_feed_and_a_separate_open_of_the_file_is_not_reported() {
    let db = TempDb::new();
    let store = Store::from_backend(open(&db).await);
    let clone = store.clone();
    let mut changes = subscribe(&store).await;
    create(&clone, "through-the-clone").await;
    let separate = Store::from_backend(open(&db).await);
    create(&separate, "through-a-separate-open").await;
    let mut its_own = subscribe(&separate).await;
    create(&store, "marker").await;
    assert_eq!(
        until(&mut changes, &instance("marker")).await,
        vec![ActorChange::changed(instance("through-the-clone"))]
    );
    assert!(store.get(&instance("through-a-separate-open")).await.unwrap().is_some(), "only the hint is missing");
    create(&separate, "marker-2").await;
    assert_eq!(until(&mut its_own, &instance("marker-2")).await, vec![]);
    separate.close().await;
    store.close().await;
}

// ---- resumable close ----

/// Starts a create that stops before its commit while it holds the writer, and returns its task once it got there.
async fn held_writer(store: &Arc<SqliteStore>) -> tokio::task::JoinHandle<Result<CreateResult, StoreError>> {
    let reached = Arc::new(Notify::new());
    *store.commit_fault.lock().unwrap() = Some(CommitFault::BeforeCommit(reached.clone()));
    let task = {
        let store = store.clone();
        tokio::spawn(async move { store.create(&instance("held"), json!({}), None).await })
    };
    reached.notified().await;
    task
}

/// Cancels the create of [`held_writer`]: its transaction never asked to commit, so it frees the writer at once.
async fn release(task: tokio::task::JoinHandle<Result<CreateResult, StoreError>>) {
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

/// True when the writer slot is empty; the writer lock must be free.
fn writer_taken(store: &SqliteStore) -> bool {
    store.writer.try_lock().expect("nothing holds the writer").is_none()
}

fn idle_readers(store: &SqliteStore) -> usize {
    store.readers.lock().unwrap().len()
}

/// Asserts that `store` refuses work and that its change streams, existing and new, end.
async fn assert_closed(store: &SqliteStore, changes: &mut ActorChanges) {
    assert_eq!(next(changes).await, None);
    assert_eq!(next(&mut ActorChanges::new(store.changes(), "")).await, None);
    assert!(store.get(&instance("a")).await.unwrap_err().is_unavailable());
    assert!(store.create(&instance("b"), json!({}), None).await.unwrap_err().is_unavailable());
}

#[tokio::test]
async fn a_close_cancelled_while_it_waits_for_the_writer_is_finished_by_a_later_close() {
    let db = TempDb::new();
    let store = open(&db).await;
    let mut changes = subscribe(&*store).await;
    let blocker = held_writer(&store).await;
    {
        let mut first = std::pin::pin!(store.close());
        assert!(futures::poll!(first.as_mut()).is_pending(), "the create holds the writer");
    }
    assert_closed(&store, &mut changes).await;
    assert_eq!(idle_readers(&store), super::READERS, "the cancelled close took no connection");
    let mut second = std::pin::pin!(store.close());
    assert!(futures::poll!(second.as_mut()).is_pending(), "a later close waits for the held writer");
    release(blocker).await;
    tokio::time::timeout(Duration::from_secs(10), second).await.expect("closes once the writer is free");
    assert!(writer_taken(&store));
    assert_eq!(idle_readers(&store), 0);
    tokio::time::timeout(Duration::from_secs(10), store.close()).await.expect("a closed store closes at once");
}

#[tokio::test]
async fn a_waiting_close_takes_over_from_a_cancelled_one_and_both_wait_for_the_writer() {
    let db = TempDb::new();
    let store = open(&db).await;
    let blocker = held_writer(&store).await;
    let mut first = Box::pin(store.close());
    let mut second = Box::pin(store.close());
    assert!(futures::poll!(first.as_mut()).is_pending());
    assert!(futures::poll!(second.as_mut()).is_pending(), "the closed flag alone does not finish a close");
    drop(first);
    assert!(futures::poll!(second.as_mut()).is_pending(), "the second close now waits for the writer");
    release(blocker).await;
    tokio::time::timeout(Duration::from_secs(10), second).await.expect("closes once the writer is free");
    assert!(writer_taken(&store));
    assert_eq!(idle_readers(&store), 0);
}

/// Starts a close that stops for good right after it took `connection` (0: the writer, then the readers in pop order)
/// for its own close, and cancels it there.
/// The close runs in the caller's future, without a task of its own, and is dropped there.
async fn close_cancelled_after_taking(store: &SqliteStore, connection: usize) {
    let reached = Arc::new(Notify::new());
    *store.close_hooks.taken.lock().unwrap() = Some((connection, reached.clone()));
    let mut close = std::pin::pin!(store.close());
    let stopped = async {
        tokio::select! {
            biased;
            () = reached.notified() => {}
            () = &mut close => panic!("the close stops after taking connection {connection}"),
        }
    };
    tokio::time::timeout(Duration::from_secs(10), stopped).await.expect("the close took the connection in time");
}

/// Cancelled after the writer, or after one or two readers, were taken for their own close: the connection it held drops
/// into the driver's cleanup, every untouched reader stays in the pool, and a later close closes them.
#[tokio::test]
async fn a_close_cancelled_while_it_closes_a_connection_leaves_the_untouched_ones_to_a_later_close() {
    for (taken, left) in [(0, super::READERS), (1, super::READERS - 1), (2, super::READERS - 2)] {
        let db = TempDb::new();
        let store = open(&db).await;
        close_cancelled_after_taking(&store, taken).await;
        assert!(writer_taken(&store), "after {taken}");
        assert_eq!(idle_readers(&store), left, "after {taken}: the readers close did not take stay in the pool");
        tokio::time::timeout(Duration::from_secs(10), store.close()).await.expect("the later close finishes");
        assert_eq!(idle_readers(&store), 0);
    }
}

/// Fails the test after 10 s of waiting for `condition`, which another thread makes true; yields between checks.
fn wait_for(what: &str, condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::yield_now();
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}

/// One close stops right after it set the closed flag; another overtakes it there. The overtaking close does not return
/// because the flag is set: it runs the whole prefix (hub, reader permits) and the cleanup itself.
#[test]
fn a_close_overtaking_another_after_its_flag_runs_the_whole_stop_and_cleanup() {
    let runtime = runtime();
    let db = TempDb::new();
    let store = runtime.block_on(open(&db));
    let mut changes = runtime.block_on(subscribe(&*store));
    let (reached, go) = store.close_hooks.after_flag.arm();
    std::thread::scope(|s| {
        let go = go; // dropped if the test panics, so a paused thread goes on
        let first = s.spawn(|| runtime.block_on(store.close()));
        reached.recv_timeout(Duration::from_secs(10)).expect("the first close set the flag");
        assert!(store.closed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!store.hub.is_closed(), "the first close stopped before the rest of its prefix");
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), store.close()).await.expect("the overtaking close finishes");
            assert!(store.hub.is_closed());
            assert!(store.reader_permits.is_closed());
            assert!(writer_taken(&store));
            assert_eq!(idle_readers(&store), 0);
            assert_closed(&store, &mut changes).await;
        });
        go.send(()).unwrap();
        first.join().unwrap();
    });
}

/// A reader stops on its way back before it takes the readers' lock, and close runs to its end meanwhile: the reader
/// then reads the flag under the lock and drops its connection. (With the flag read before the lock, as before this
/// change, the same order put the connection back into the drained pool.)
#[test]
fn a_reader_returning_after_close_drained_the_pool_drops_its_connection() {
    let runtime = runtime();
    let db = TempDb::new();
    let store = runtime.block_on(open(&db));
    let read = runtime.block_on(store.read()).unwrap();
    let (reached, go) = store.close_hooks.return_before_lock.arm();
    std::thread::scope(|s| {
        let go = go; // dropped if the test panics, so a paused thread goes on
        let returning = s.spawn(|| drop(read));
        reached.recv_timeout(Duration::from_secs(10)).expect("the reader is on its way back");
        let close = runtime.block_on(async { tokio::time::timeout(Duration::from_secs(10), store.close()).await });
        close.expect("close does not wait");
        assert_eq!(idle_readers(&store), 0);
        go.send(()).unwrap();
        returning.join().unwrap();
    });
    assert_eq!(idle_readers(&store), 0, "the returned connection is dropped, not pooled");
}

/// A reader read the open flag under the readers' lock and stops there, about to put its connection back; close sets
/// the flag meanwhile and must then wait for the lock, so its drain comes after the return and takes that connection.
#[test]
fn a_reader_returning_under_the_lock_before_the_drain_is_drained_with_the_pool() {
    let runtime = runtime();
    let db = TempDb::new();
    let store = runtime.block_on(open(&db));
    let read = runtime.block_on(store.read()).unwrap();
    let (reached, go) = store.close_hooks.return_under_lock.arm();
    std::thread::scope(|s| {
        let go = go; // dropped if the test panics, so a paused thread goes on
        let returning = s.spawn(|| drop(read));
        reached.recv_timeout(Duration::from_secs(10)).expect("the reader read the flag under the lock");
        let close = s.spawn(|| runtime.block_on(store.close()));
        wait_for("close to set the flag", || store.closed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!close.is_finished(), "the drain waits for the readers' lock");
        go.send(()).unwrap();
        returning.join().unwrap();
        close.join().unwrap();
    });
    assert_eq!(idle_readers(&store), 0, "the drain took the returned connection");
}

/// After a close cancelled while it held the writer for its own close, the store's last handle is dropped, or first its
/// runtime is shut down: the untouched readers are still the store's, no task was spawned for them, and dropping the
/// store frees it (driver cleanup of the connections is not awaited).
#[test]
fn a_cancelled_close_leaves_its_resources_to_the_final_drop_or_the_runtime_teardown_without_a_task() {
    for teardown in [false, true] {
        let runtime = runtime();
        let db = TempDb::new();
        let store = runtime.block_on(open(&db));
        let mut changes = runtime.block_on(subscribe(&*store));
        let tasks = runtime.metrics().num_alive_tasks();
        runtime.block_on(close_cancelled_after_taking(&store, 0));
        assert_eq!(runtime.metrics().num_alive_tasks(), tasks, "the cancelled close leaves no task behind");
        assert_eq!(idle_readers(&store), super::READERS, "the untouched readers are retained");
        runtime.block_on(async { assert_eq!(next(&mut changes).await, None) });
        if teardown {
            drop(runtime);
        } else {
            assert_eq!(runtime.metrics().num_alive_tasks(), tasks);
        }
        assert_eq!(Arc::strong_count(&store), 1, "nothing but the test holds the store");
        let weak = Arc::downgrade(&store);
        drop(store);
        assert!(weak.upgrade().is_none(), "the final drop frees the store and its connections");
    }
}

#[tokio::test]
async fn a_close_future_that_is_never_polled_has_no_effect() {
    let db = TempDb::new();
    let store = open(&db).await;
    drop(store.close());
    let mut changes = subscribe(&*store).await;
    create(&*store, "after").await;
    assert_eq!(next(&mut changes).await, Some(ActorChange::changed(instance("after"))));
    assert!(!writer_taken(&store));
    store.close().await;
}

#[tokio::test]
async fn a_reader_held_across_close_returns_to_no_pool() {
    let db = TempDb::new();
    let store = open(&db).await;
    let read = store.read().await.unwrap();
    assert_eq!(idle_readers(&store), super::READERS - 1);
    tokio::time::timeout(Duration::from_secs(10), store.close()).await.expect("close does not wait for reads");
    assert_eq!(idle_readers(&store), 0);
    drop(read);
    assert_eq!(idle_readers(&store), 0, "the returned connection is dropped");
}
