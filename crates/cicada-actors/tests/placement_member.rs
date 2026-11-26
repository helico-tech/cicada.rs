//! The member of a placement pool: the identity file in a temp folder (made once, read the same after a restart, and
//! an unreadable or invalid file refused and left as it is), and the member's rounds on virtual time with `TestActors`
//! (offers while the probe answers, none when it does not, the node's own id, a failed send tried again, the stop also
//! while the probe hangs, and a zero interval refused).

use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::runtime::NodeConfig;
use cicada::store::NodeId;
use cicada::time;
use cicada_actors::{
    Capacity, IdentityErrorKind, MemberSettings, NodeIdentity, Placement, PlacementActor, PlacementMember,
    PlacementNodeStatus, PlacementState,
};
use cicada_test::TestActors;
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn pool() -> Placement {
    Placement::named("runners")
}

/// Scenario: A restart. The first load makes the missing folders and the file; the next reads the same identity.
#[test]
fn the_identity_is_made_once_and_read_the_same_after_a_restart() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path().join("var").join("placement");
    let first = NodeIdentity::load_or_create(&dir).unwrap();
    let node = first.node_id();
    let hex = node.as_str().strip_prefix("node-").unwrap();
    assert!(hex.len() == 16 && hex.chars().all(|c| c.is_ascii_hexdigit()), "{node}");
    assert!(!first.instance().is_empty());
    let file: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("placement-identity.json")).unwrap()).unwrap();
    assert_eq!(file, serde_json::json!({"node": node.as_str(), "instance": first.instance()}));
    let instance = first.instance().to_owned();
    drop(first);
    let restarted = NodeIdentity::load_or_create(&dir).unwrap();
    assert_eq!((restarted.node_id(), restarted.instance()), (node.clone(), instance.as_str()), "the same identity");
    let mut names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    assert_eq!(names, ["placement-identity.json", "placement-identity.lock"], "no temporary file left");
    let other = NodeIdentity::load_or_create(temp.path().join("other")).unwrap();
    assert!(other.node_id() != node && other.instance() != instance, "another folder, another identity");
}

/// Scenario: A second process. While the identity, a clone of it or a member that holds it lives, a second load of the
/// folder fails with `Locked` and changes nothing; once they are all dropped the folder loads again.
#[test]
fn a_second_load_fails_while_the_identity_lives_and_succeeds_after_all_are_dropped() {
    let temp = TempDir::new().unwrap();
    let first = NodeIdentity::load_or_create(temp.path()).unwrap();
    let file = fs::read(temp.path().join("placement-identity.json")).unwrap();
    let locked = |why: &str| {
        let e = NodeIdentity::load_or_create(temp.path()).unwrap_err();
        assert_eq!(e.kind(), IdentityErrorKind::Locked, "{why}: {e}");
        assert!(e.to_string().contains("placement-identity.lock"), "{e}");
    };
    locked("the identity lives");
    let clone = first.clone();
    assert_eq!(clone, first, "a clone is equal and shares the lock");
    drop(first);
    locked("a clone lives");
    let member = PlacementMember::new(pool(), clone, MemberSettings::new(), || async { None });
    locked("the member holds it");
    drop(member);
    let again = NodeIdentity::load_or_create(temp.path()).unwrap();
    assert_eq!(fs::read(temp.path().join("placement-identity.json")).unwrap(), file, "the file is unchanged");
    drop(again);
    let e = {
        fs::write(temp.path().join("placement-identity.json"), "not json").unwrap();
        NodeIdentity::load_or_create(temp.path()).unwrap_err()
    };
    assert_eq!(e.kind(), IdentityErrorKind::Invalid, "{e}");
}

/// Scenario: A corrupt identity file. Text that is not the identity JSON, or ids out of bounds, are refused and the
/// file stays as it is.
#[test]
fn an_invalid_identity_file_is_refused_and_left_as_it_is() {
    for text in [
        "not json",
        r#"{"node": "n1"}"#,
        r#"{"node": "n1", "instance": "i1", "extra": 1}"#,
        r#"{"node": "n1", "instance": " "}"#,
        r#"{"node": "", "instance": "i1"}"#,
    ] {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("placement-identity.json");
        fs::write(&path, text).unwrap();
        let e = NodeIdentity::load_or_create(temp.path()).unwrap_err();
        assert!(e.to_string().contains("placement-identity.json"), "{text}: {e}");
        assert_eq!(e.kind(), IdentityErrorKind::Invalid);
        assert_eq!(fs::read_to_string(&path).unwrap(), text, "left as it is");
    }
}

/// A file that cannot be read, here a folder in its place, is refused, never replaced.
#[test]
fn an_unreadable_identity_file_is_refused() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("placement-identity.json")).unwrap();
    let e = NodeIdentity::load_or_create(temp.path()).unwrap_err();
    assert!(e.to_string().contains("cannot be read"), "{e}");
    assert!(temp.path().join("placement-identity.json").is_dir());
}

/// A read-only folder that holds the identity file still loads it. With the lock file missing (it cannot be created)
/// it goes on without a lock, with a warning; with the lock file there but read-only it is opened read-only and locked,
/// so a second load is refused while the first lives.
#[cfg(unix)]
#[test]
fn a_read_only_folder_loads_the_identity_and_locks_a_read_only_lock_file() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let dir = temp.path().join("placement");
    let made = NodeIdentity::load_or_create(&dir).unwrap();
    let (node, instance) = (made.node_id(), made.instance().to_owned());
    drop(made);
    let lock = dir.join("placement-identity.lock");
    let mode = |path: &std::path::Path, mode: u32| fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    for missing in [true, false] {
        if missing {
            fs::remove_file(&lock).unwrap();
        } else {
            mode(&dir, 0o755);
            fs::write(&lock, "").unwrap();
            mode(&lock, 0o444);
        }
        mode(&dir, 0o555);
        let (read, created) = (NodeIdentity::load_or_create(&dir), lock.exists() && missing);
        let again = NodeIdentity::load_or_create(&dir);
        mode(&dir, 0o755);
        let read = read.unwrap_or_else(|e| panic!("missing lock file {missing}: {e}"));
        assert_eq!((read.node_id(), read.instance()), (node.clone(), instance.as_str()), "{missing}");
        assert!(!created, "nothing created");
        if missing {
            assert!(format!("{read:?}").contains("locked: false"), "{read:?}");
            assert!(again.is_ok(), "no lock held, so a second load is not refused: {again:?}");
        } else {
            assert!(format!("{read:?}").contains("locked: true"), "{read:?}");
            let e = again.unwrap_err();
            assert_eq!(e.kind(), IdentityErrorKind::Locked, "{e}");
            drop(read);
            mode(&dir, 0o555);
            let after = NodeIdentity::load_or_create(&dir);
            mode(&dir, 0o755);
            assert!(after.is_ok(), "the lock is released with the identity: {after:?}");
        }
    }
}

/// Scenario: A zero interval.
#[test]
fn a_zero_interval_is_refused() {
    assert_eq!(MemberSettings::new().interval(), secs(15));
    let e = MemberSettings::new().with_interval(Duration::ZERO).unwrap_err();
    assert_eq!(e.to_string(), "a member's interval is zero");
    assert_eq!(MemberSettings::new().with_interval(secs(5)).unwrap().interval(), secs(5));
}

/// A node with the pool type under the node id `from-flag`, as `--node-id` sets it; with the pool `runners` (silence
/// 60 s) when `with_pool`.
async fn node(with_pool: bool) -> TestActors {
    let config = NodeConfig { node_id: NodeId::new("from-flag").unwrap(), ..NodeConfig::default() };
    let test = TestActors::builder()
        .start_at("2026-01-01T00:00:00Z".parse().unwrap())
        .with(move |node| node.config(config).host(PlacementActor))
        .build()
        .await
        .unwrap();
    if with_pool {
        create_pool(&test).await;
    }
    test
}

async fn create_pool(test: &TestActors) {
    test.create(&pool(), &PlacementState::new(secs(60), 10, 10, 10).unwrap()).await;
}

fn identity(temp: &TempDir) -> NodeIdentity {
    NodeIdentity::load_or_create(temp.path()).unwrap()
}

/// Runs `member` on the test's node in a task of its own until the returned sender is used or dropped.
fn spawn<P, F>(test: &TestActors, member: PlacementMember<P>) -> (oneshot::Sender<()>, JoinHandle<()>)
where
    P: FnMut() -> F + Send + 'static,
    F: Future<Output = Option<Capacity>> + Send + 'static,
{
    let (stop, stopped) = oneshot::channel::<()>();
    let system = test.system().clone();
    let run = tokio::spawn(member.run(system, async move {
        let _ = stopped.await;
    }));
    (stop, run)
}

/// A probe that answers `answers` times with four places and then nothing; it counts its calls.
fn probe(answers: u32, calls: Arc<AtomicU32>) -> impl FnMut() -> std::future::Ready<Option<Capacity>> + Send {
    move || {
        let call = calls.fetch_add(1, Ordering::SeqCst);
        std::future::ready((call < answers).then(|| Capacity::new(["docker", "arm64", "docker"], 4)))
    }
}

/// Scenario: A runtime that stops answering, and a node id from the command line. Rounds at 0, 15 and 30 s offer
/// under the node's own id and the identity's instance; the later rounds send nothing, and the pool marks the node
/// silent.
#[tokio::test(start_paused = true)]
async fn the_member_offers_while_the_probe_answers_and_the_node_goes_silent_after() {
    let test = node(true).await;
    let start = test.now();
    let temp = TempDir::new().unwrap();
    let identity = identity(&temp);
    let calls = Arc::new(AtomicU32::new(0));
    let member = PlacementMember::new(pool(), identity.clone(), MemberSettings::new(), probe(3, calls.clone()));
    let (stop, run) = spawn(&test, member);
    // The first offer runs at 0 s, the later ones 1 s after their rounds: `heardAt` is the store time of the turn.
    for _ in 0..100 {
        if !test.pending(&pool()).await.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    test.run_until_idle().await;
    let mut heard = Vec::new();
    test.advance_time_by(secs(1)).await;
    for _ in 0..8 {
        let state: PlacementState = test.state(&pool()).await;
        heard.push(time::between(start, state.nodes["from-flag"].heard_at).as_secs());
        test.advance_time_by(secs(15)).await;
    }
    assert_eq!(heard, [0, 16, 31, 31, 31, 31, 31, 31], "three offers, then none");
    assert_eq!(calls.load(Ordering::SeqCst), 9, "a round every 15 s, 0 through 120 s");
    let state: PlacementState = test.state(&pool()).await;
    assert_eq!(state.nodes.keys().collect::<Vec<_>>(), ["from-flag"], "the node's own id, not the file's");
    let entry = &state.nodes["from-flag"];
    assert_eq!(entry.instance, identity.instance());
    assert_eq!((entry.labels.clone(), entry.places), (vec!["arm64".into(), "docker".into()], 4));
    assert_eq!(entry.status, PlacementNodeStatus::Silent, "silent 60 s after the last offer");
    stop.send(()).unwrap();
    run.await.unwrap();
}

/// A send that fails, to a pool not created yet, is tried again in the next round.
#[tokio::test(start_paused = true)]
async fn a_failed_send_is_tried_again_in_the_next_round() {
    let test = node(false).await;
    let start = test.now();
    let temp = TempDir::new().unwrap();
    let calls = Arc::new(AtomicU32::new(0));
    let member = PlacementMember::new(pool(), identity(&temp), MemberSettings::new(), probe(u32::MAX, calls.clone()));
    let (stop, run) = spawn(&test, member);
    test.advance_time_by(secs(1)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the first round ran and its send failed");
    assert!(!test.exists(&pool()).await);
    create_pool(&test).await;
    test.advance_time_by(secs(15)).await;
    let state: PlacementState = test.state(&pool()).await;
    let heard = time::between(start, state.nodes["from-flag"].heard_at);
    assert_eq!(heard, secs(16), "offered at the next round, 15 s, and its turn ran at 16 s");
    stop.send(()).unwrap();
    run.await.unwrap();
}

/// `stop` ends `run` between rounds, and also while a probe hangs.
#[tokio::test(start_paused = true)]
async fn the_stop_ends_the_member_also_while_the_probe_hangs() {
    let test = node(true).await;
    let temp = TempDir::new().unwrap();
    let calls = Arc::new(AtomicU32::new(0));
    let member = PlacementMember::new(pool(), identity(&temp), MemberSettings::new(), probe(u32::MAX, calls.clone()));
    let (stop, run) = spawn(&test, member);
    test.advance_time_by(secs(5)).await;
    stop.send(()).unwrap();
    tokio::time::timeout(secs(1), run).await.expect("ended between rounds").unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let probed = Arc::new(Mutex::new(0));
    let seen = probed.clone();
    let hanging = move || {
        *seen.lock().unwrap() += 1;
        std::future::pending::<Option<Capacity>>()
    };
    let member = PlacementMember::new(pool(), identity(&temp), MemberSettings::new(), hanging);
    let (stop, run) = spawn(&test, member);
    test.advance_time_by(secs(30)).await;
    assert_eq!(*probed.lock().unwrap(), 1, "the first probe still hangs");
    stop.send(()).unwrap();
    tokio::time::timeout(secs(1), run).await.expect("ended while the probe hangs").unwrap();
}

/// A capacity with a resource key offers it, and the pool records it on the node; a key out of bounds sends no offer.
#[tokio::test(start_paused = true)]
async fn a_capacity_with_a_resource_key_offers_it() {
    let test = node(true).await;
    let temp = TempDir::new().unwrap();
    let key = Arc::new(Mutex::new("daemon-1".to_owned()));
    let probed = key.clone();
    let member = PlacementMember::new(pool(), identity(&temp), MemberSettings::new(), move || {
        let key = probed.lock().unwrap().clone();
        std::future::ready(Some(Capacity::new(["arm64"], 2).resource(key)))
    });
    assert_eq!(Capacity::new(["arm64"], 2).resource("daemon-1").resource.as_deref(), Some("daemon-1"));
    assert_eq!(Capacity::new(["arm64"], 2).resource, None);
    let (stop, run) = spawn(&test, member);
    tokio::task::yield_now().await;
    test.run_until_idle().await;
    let state: PlacementState = test.state(&pool()).await;
    assert_eq!(state.nodes["from-flag"].resource.as_deref(), Some("daemon-1"));
    *key.lock().unwrap() = String::new();
    test.advance_time_by(secs(15)).await;
    tokio::task::yield_now().await;
    test.run_until_idle().await;
    let later: PlacementState = test.state(&pool()).await;
    assert_eq!(later.nodes["from-flag"].heard_at, state.nodes["from-flag"].heard_at, "an empty key: no offer");
    stop.send(()).unwrap();
    run.await.unwrap();
}
