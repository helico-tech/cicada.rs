//! Example 18: a job that owns a sandbox. Finished, it releases its capacity and runs no cleanup. Cancelled, its cleanup
//! stops the sandbox and waits for the removal before it releases the capacity, reports in a turn before the one that
//! marks it cancelled, releases a reservation whose result never committed by its key, and relies on its owners to
//! fence a late start and a late reserve; a reply that comes between cleanup turns does not change the branch taken.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada_actors::{FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, JournalEntry};
use cicada_examples::flow_resources::{
    Capacity, Containers, JobInput, Notices, NoticesActor, NoticesState, Sandbox, SandboxActor, SandboxJob, SandboxMsg,
    SandboxState, job,
};
use cicada_examples::steps::ServiceError;
use cicada_test::TestActors;
use std::time::Duration;

/// The fake capacity service and container runtime: what is reserved, what was released, what containers run, and
/// every call as `<what> <key>`.
#[derive(Clone, Default)]
struct Fakes {
    reserved: Arc<Mutex<BTreeSet<String>>>,
    released: Arc<Mutex<BTreeSet<String>>>,
    containers: Arc<Mutex<BTreeSet<String>>>,
    calls: Arc<Mutex<Vec<String>>>,
    /// The next reserve reserves, then fails: its effect happened, its result is never saved.
    fail_after_reserve: Arc<AtomicBool>,
    /// The next create fails.
    fail_create: Arc<AtomicBool>,
}

impl Fakes {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn note(&self, what: &str, key: &str) {
        self.calls.lock().unwrap().push(format!("{what} {key}"));
    }

    fn reserved(&self) -> Vec<String> {
        self.reserved.lock().unwrap().iter().cloned().collect()
    }

    fn containers(&self) -> Vec<String> {
        self.containers.lock().unwrap().iter().cloned().collect()
    }
}

impl Capacity for Fakes {
    async fn reserve(&self, key: &str) -> Result<(), ServiceError> {
        self.note("reserve", key);
        if self.released.lock().unwrap().contains(key) {
            return Err(ServiceError(format!("{key} was released")));
        }
        self.reserved.lock().unwrap().insert(key.to_owned());
        if self.fail_after_reserve.swap(false, Ordering::SeqCst) {
            return Err(ServiceError("the answer was lost".into()));
        }
        Ok(())
    }

    async fn release(&self, key: &str) -> Result<(), ServiceError> {
        self.note("release", key);
        self.reserved.lock().unwrap().remove(key);
        self.released.lock().unwrap().insert(key.to_owned());
        Ok(())
    }
}

impl Containers for Fakes {
    async fn create(&self, key: &str) -> Result<(), ServiceError> {
        self.note("create", key);
        if self.fail_create.swap(false, Ordering::SeqCst) {
            return Err(ServiceError("the runtime is busy".into()));
        }
        self.containers.lock().unwrap().insert(key.to_owned());
        Ok(())
    }

    async fn remove(&self, key: &str) -> Result<(), ServiceError> {
        self.note("remove", key);
        self.containers.lock().unwrap().remove(key);
        Ok(())
    }
}

type Run = FlowRun<SandboxJob<Fakes>>;
type State = FlowState<JobInput>;

const RESERVATION: &str = "example.sandbox-job/j1/reserve";
const CONTAINER: &str = "example.sandboxes/j1";

async fn harness(fakes: &Fakes) -> TestActors {
    let (capacity, containers) = (fakes.clone(), fakes.clone());
    let test = TestActors::builder()
        .with(move |node| {
            node.host(FlowActor::new(SandboxJob { capacity })).host(SandboxActor { containers }).host(NoticesActor)
        })
        .build()
        .await
        .unwrap();
    test.create(&owner(), &NoticesState::default()).await;
    test
}

fn owner() -> Notices {
    Notices("team-a".into())
}

fn sandbox() -> Sandbox {
    Sandbox("j1".into())
}

/// Starts the job `j1` and runs it until it waits for its workload, with its sandbox running.
async fn started(test: &TestActors, fakes: &Fakes) -> Run {
    let run: Run = job("j1");
    test.create(&run, &State::new(JobInput { job: "j1".into(), owner: "team-a".into() })).await;
    test.send(&run, FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(fakes.containers(), [CONTAINER]);
    assert_eq!(test.state::<_, State>(&run).await.armed, None, "waiting for the workload, with nothing armed");
    run
}

async fn state(test: &TestActors, run: &Run) -> State {
    test.state(run).await
}

async fn notices(test: &TestActors) -> Vec<String> {
    test.state::<_, NoticesState>(&owner()).await.lines
}

fn ids(journal: &[JournalEntry]) -> Vec<&str> {
    journal.iter().map(JournalEntry::id).collect()
}

/// A job whose workload finishes releases its capacity and reports; it is done, and its cleanup never runs.
#[tokio::test(start_paused = true)]
async fn a_job_that_finishes_releases_its_capacity_and_runs_no_cleanup() {
    let fakes = Fakes::default();
    let test = harness(&fakes).await;
    let run = started(&test, &fakes).await;
    test.send(&sandbox(), SandboxMsg::Finish { output: "ok".into() }).await;
    test.run_until_idle().await;
    let done = state(&test, &run).await;
    assert_eq!(done.status, FlowStatus::Done);
    assert_eq!(ids(&done.journal), ["reserve", "start", "finished", "release", "report"]);
    assert!(fakes.reserved().is_empty() && fakes.containers().is_empty());
    assert_eq!(notices(&test).await, ["finished j1 ok"]);
    test.send(&run, FlowMsg::Cancel).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, &run).await, done, "a cancel after done changes nothing");
}

/// A job cancelled while its workload runs: the cleanup sends `Stop` and waits; while the sandbox has not confirmed,
/// the capacity stays reserved. The confirmation comes after the container is removed; the cleanup then releases the
/// capacity, and its report commits in a turn before the one that marks the run cancelled. No main code ran again.
#[tokio::test(start_paused = true)]
async fn a_cancelled_job_confirms_the_sandbox_removed_before_it_releases_capacity_and_reports_before_cancelled() {
    let fakes = Fakes::default();
    let test = harness(&fakes).await;
    let run = started(&test, &fakes).await;
    let main = state(&test, &run).await.journal;
    test.system().pause_mailbox(&sandbox().address().unwrap()).await.unwrap();
    test.send(&run, FlowMsg::Cancel).await;
    test.run_until_idle().await;
    let waiting = state(&test, &run).await;
    assert_eq!((waiting.status, ids(&waiting.cleanup)), (FlowStatus::Cancelling, vec!["stop", "stopped"]));
    assert_eq!((fakes.reserved(), fakes.containers()), (vec![RESERVATION.to_owned()], vec![CONTAINER.to_owned()]));

    test.system().resume_mailbox(&sandbox().address().unwrap()).await.unwrap();
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the stop");
    assert!(fakes.containers().is_empty(), "removed before the confirmation");
    assert_eq!(fakes.reserved(), [RESERVATION], "the capacity waits for the confirmation");
    for _ in 0..3 {
        test.system().run_due_turns(1).await.unwrap(); // the reply, the release, the report's send
    }
    let reported = state(&test, &run).await;
    assert_eq!(
        (reported.status, ids(&reported.cleanup)),
        (FlowStatus::Cancelling, vec!["stop", "stopped", "release", "report"])
    );
    assert!(fakes.reserved().is_empty());
    assert_eq!(test.pending(&owner()).await.len(), 1, "the report is sent; the run is not yet cancelled");
    test.run_until_idle().await;
    let cancelled = state(&test, &run).await;
    assert_eq!((cancelled.status, cancelled.armed), (FlowStatus::Cancelled, None));
    assert_eq!(cancelled.journal, main, "the main journal stays");
    assert_eq!(notices(&test).await, ["cancelled j1"]);
    let order: Vec<String> = fakes.calls().into_iter().filter(|c| !c.starts_with("create")).collect();
    assert_eq!(
        order,
        [format!("reserve {RESERVATION}"), format!("remove {CONTAINER}"), format!("release {RESERVATION}")]
    );
}

/// The reservation reserved, but its step failed before its result was saved: the run is cancelled while the step
/// waits for its retry. The main journal holds nothing, yet the cleanup releases the reservation by its key; no
/// sandbox was ever started, so it stops none. A late reserve of the key, from a step still in flight, is refused.
#[tokio::test(start_paused = true)]
async fn a_reservation_whose_result_never_committed_is_released_by_its_key() {
    let fakes = Fakes::default();
    fakes.fail_after_reserve.store(true, Ordering::SeqCst);
    let test = harness(&fakes).await;
    let run: Run = job("j1");
    test.create(&run, &State::new(JobInput { job: "j1".into(), owner: "team-a".into() })).await;
    test.send(&run, FlowMsg::Continue).await;
    test.run_until_idle().await;
    assert_eq!(fakes.reserved(), [RESERVATION], "the reservation happened");
    let held = state(&test, &run).await;
    assert!(held.journal.is_empty(), "its result was not saved");
    test.send(&run, FlowMsg::Cancel).await;
    test.run_until_idle().await;
    let cancelled = state(&test, &run).await;
    assert_eq!((cancelled.status, ids(&cancelled.cleanup)), (FlowStatus::Cancelled, vec!["release", "report"]));
    assert!(fakes.reserved().is_empty(), "released by its key");
    assert!(!test.exists(&sandbox()).await, "no sandbox, no stop");
    assert!(fakes.reserve(RESERVATION).await.is_err(), "a late reserve is fenced");
    assert!(fakes.reserved().is_empty());
    test.advance_time_by(Duration::from_secs(60)).await;
    assert_eq!(fakes.calls().iter().filter(|c| c.starts_with("reserve")).count(), 2, "the step never ran again");
}

/// The sandbox's start failed, and the run is cancelled: the cleanup stops the sandbox, which fences later starts, and
/// releases the capacity. An operator's retry of the failed start then creates nothing.
#[tokio::test(start_paused = true)]
async fn a_late_start_after_the_stop_is_fenced() {
    let fakes = Fakes::default();
    fakes.fail_create.store(true, Ordering::SeqCst);
    let test = harness(&fakes).await;
    let run: Run = job("j1");
    test.create(&run, &State::new(JobInput { job: "j1".into(), owner: "team-a".into() })).await;
    test.send(&run, FlowMsg::Continue).await;
    test.run_until_idle().await;
    let start = match &state(&test, &run).await.journal[1] {
        JournalEntry::Send { message, .. } => message.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(test.system().outcome(&start).await.unwrap().unwrap().status, OutcomeStatus::Failed);
    test.send(&run, FlowMsg::Cancel).await;
    test.run_until_idle().await;
    assert_eq!(state(&test, &run).await.status, FlowStatus::Cancelled);
    assert!(test.state::<_, SandboxState>(&sandbox()).await.stopped);

    let cicada::runtime::RetryResult::Accepted(_) = test.system().retry_message(&start).await.unwrap() else {
        panic!("retried")
    };
    test.run_until_idle().await;
    assert!(fakes.containers().is_empty(), "the late start created nothing");
    assert_eq!(fakes.calls().iter().filter(|c| c.starts_with("create")).count(), 1, "only the failed attempt");
    assert!(fakes.reserved().is_empty());
}

/// The workload's `Finished` reply comes after the `Cancel`, between two cleanup turns. It is kept, but the main wait
/// stays open, so `main_reply` still answers open: the cleanup goes on with the branch it took, waits for `Stopped`,
/// and releases the capacity. Had the main wait decided before the `Cancel`, the cleanup would only release.
#[tokio::test(start_paused = true)]
async fn a_reply_between_cleanup_turns_does_not_change_the_branch_and_a_decided_one_skips_the_stop() {
    let fakes = Fakes::default();
    let test = harness(&fakes).await;
    let run = started(&test, &fakes).await;
    let address = sandbox().address().unwrap();
    test.system().pause_mailbox(&address).await.unwrap();
    test.send(&sandbox(), SandboxMsg::Finish { output: "late".into() }).await;
    test.send(&run, FlowMsg::Cancel).await;
    test.run_until_idle().await;
    assert_eq!(ids(&state(&test, &run).await.cleanup), ["stop", "stopped"], "the open branch, taken");
    test.system().resume_mailbox(&address).await.unwrap();
    test.run_until_idle().await;
    let cancelled = state(&test, &run).await;
    assert_eq!(cancelled.status, FlowStatus::Cancelled);
    assert_eq!(ids(&cancelled.cleanup), ["stop", "stopped", "release", "report"], "the same branch, replayed");
    assert_eq!(cancelled.replies.len(), 2, "the late Finished and the Stopped are both kept");
    assert!(
        matches!(cancelled.journal.last(), Some(JournalEntry::Wait { outcome: None, .. })),
        "the main wait stays open"
    );
    assert!(fakes.reserved().is_empty() && fakes.containers().is_empty());

    let fakes = Fakes::default();
    let test = harness(&fakes).await;
    let run = started(&test, &fakes).await;
    test.system().pause_mailbox(&run.address().unwrap()).await.unwrap();
    test.send(&sandbox(), SandboxMsg::Finish { output: "ok".into() }).await;
    test.run_until_idle().await;
    test.system().resume_mailbox(&run.address().unwrap()).await.unwrap();
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1, "the reply closes the main wait");
    test.send(&run, FlowMsg::Cancel).await; // behind the execution the reply armed, which saves the main release
    test.run_until_idle().await;
    let ended = state(&test, &run).await;
    assert_eq!(ended.status, FlowStatus::Cancelled);
    assert_eq!(ids(&ended.journal), ["reserve", "start", "finished", "release"]);
    assert_eq!(ids(&ended.cleanup), ["release", "report"], "a decided start needs no stop; the release is idempotent");
    assert!(fakes.reserved().is_empty());
    assert_eq!(notices(&test).await, ["cancelled j1"]);
}
