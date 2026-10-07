//! Example 3: the cron job.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::store::OutcomeStatus;
use cicada::time::{self, Timestamp};
use cicada_examples::cron::{CronJob, CronJobActor, CronMsg, CronState, CronTask, TaskError};
use cicada_extras::Schedule;
use cicada_test::TestActors;

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(24 * 3600);

/// How the fake task behaves.
#[derive(Clone, Copy)]
enum Behaviour {
    /// Records the slot.
    Record,
    /// The second call fails; the others record the slot.
    FailSecondCall,
    /// Records the slot, then works 15 minutes (within the maximum turn).
    Slow,
    /// The first call works an hour (past the maximum turn of 30 minutes); then records the slot.
    StuckFirstCall,
}

#[derive(Clone)]
struct FakeTask {
    behaviour: Behaviour,
    runs: Arc<Mutex<Vec<Timestamp>>>,
    calls: Arc<Mutex<u32>>,
}

impl FakeTask {
    fn new(behaviour: Behaviour) -> Self {
        Self { behaviour, runs: Arc::default(), calls: Arc::default() }
    }

    fn runs(&self) -> Vec<Timestamp> {
        self.runs.lock().unwrap().clone()
    }
}

impl CronTask for FakeTask {
    async fn run(&self, _job: &str, scheduled_for: Timestamp) -> Result<(), TaskError> {
        let calls = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        match self.behaviour {
            Behaviour::Record => {}
            Behaviour::FailSecondCall if calls == 2 => return Err("report service down".into()),
            Behaviour::FailSecondCall => {}
            Behaviour::Slow => {
                self.runs.lock().unwrap().push(scheduled_for);
                tokio::time::sleep(15 * MINUTE).await; // virtual time; within MAX_TURN
                return Ok(());
            }
            Behaviour::StuckFirstCall if calls == 1 => tokio::time::sleep(HOUR).await, // cut off at MAX_TURN
            Behaviour::StuckFirstCall => {}
        }
        self.runs.lock().unwrap().push(scheduled_for);
        Ok(())
    }
}

fn job() -> CronJob {
    CronJob { name: "hourly-report".into() }
}

async fn harness(task: &FakeTask, schedule: Schedule) -> TestActors {
    let task = task.clone();
    let test = TestActors::builder().with(move |node| node.host(CronJobActor { task })).build().await.unwrap();
    test.create(&job(), &CronState::new(schedule)).await;
    test
}

async fn state(test: &TestActors) -> CronState {
    test.state(&job()).await
}

/// Starts the job and returns its first slot.
async fn start(test: &TestActors) -> Timestamp {
    test.send(&job(), CronMsg::Start).await;
    test.run_until_idle().await;
    state(test).await.next_run.expect("armed")
}

fn plus(t: Timestamp, d: Duration) -> Timestamp {
    time::add(t, d).unwrap()
}

#[tokio::test(start_paused = true)]
async fn each_tick_does_the_work_and_schedules_the_next_one() {
    let task = FakeTask::new(Behaviour::Record);
    let hourly = Schedule::every(3600).unwrap();
    let test = harness(&task, hourly).await;
    let first = start(&test).await;
    assert_eq!(first, hourly.next(test.now()));

    test.advance_time_by(5 * HOUR).await;
    assert_eq!(task.runs(), (0..5).map(|i| plus(first, i * HOUR)).collect::<Vec<_>>());
    let s = state(&test).await;
    assert_eq!(s.runs, 5);
    assert_eq!(s.next_run, Some(plus(first, 5 * HOUR)));
    assert_eq!(test.pending(&job()).await.len(), 1); // always exactly one tick armed
}

#[tokio::test(start_paused = true)]
async fn starting_twice_does_not_double_the_chain() {
    let task = FakeTask::new(Behaviour::Record);
    let test = harness(&task, Schedule::daily(3, 0).unwrap()).await;
    test.send(&job(), CronMsg::Start).await;
    test.send(&job(), CronMsg::Start).await;
    test.advance_time_by(3 * DAY).await;
    let runs = task.runs();
    assert_eq!(runs.len(), 3);
    assert_eq!(test.pending(&job()).await.len(), 1);
    let gaps: Vec<Duration> = runs.windows(2).map(|w| time::between(w[0], w[1])).collect();
    assert_eq!(gaps, vec![DAY, DAY]);
}

#[tokio::test(start_paused = true)]
async fn a_stray_tick_is_ignored_and_stopping_ends_the_chain() {
    let task = FakeTask::new(Behaviour::Record);
    let test = harness(&task, Schedule::every(600).unwrap()).await;
    start(&test).await;
    test.send(&job(), CronMsg::Tick { scheduled_for: test.now() }).await; // not the armed slot
    test.run_until_idle().await;
    assert!(task.runs().is_empty());

    test.advance_time_by(30 * MINUTE).await;
    assert_eq!(task.runs().len(), 3);
    test.send(&job(), CronMsg::StopJob).await;
    test.run_until_idle().await;
    assert!(!test.exists(&job()).await);
    assert_eq!(task.runs().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_task_that_fails_once_does_not_end_the_chain() {
    let task = FakeTask::new(Behaviour::FailSecondCall);
    let test = harness(&task, Schedule::every(3600).unwrap()).await;
    let first = start(&test).await;
    // The first tick, and the second, whose task fails.
    test.advance_time_by(time::between(test.now(), first) + HOUR).await;
    let s = state(&test).await;
    assert_eq!(s.runs, 2);
    assert_eq!(s.failed_runs, 1);
    assert_eq!(s.last_error.as_deref(), Some("report service down"));
    assert_eq!(s.next_run, Some(plus(first, 2 * HOUR)), "the next tick is armed after the failed one");
    assert!(test.failures().await.is_empty(), "the tick itself did not fail");

    test.advance_time_by(HOUR).await;
    assert_eq!(task.runs(), vec![first, plus(first, 2 * HOUR)]);
    assert_eq!(state(&test).await.last_error, None, "cleared by the next successful run");
    assert_eq!(test.pending(&job()).await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_task_longer_than_its_slot_skips_the_slots_it_overran() {
    let task = FakeTask::new(Behaviour::Slow);
    let test = harness(&task, Schedule::every(600).unwrap()).await;
    let first = start(&test).await;
    test.advance_time_by(time::between(test.now(), first) + 16 * MINUTE).await;
    // The next slot after the work ended (first + 15 min), not after the claim: first + 20 min, not + 10 min.
    assert_eq!(state(&test).await.next_run, Some(plus(first, 20 * MINUTE)));
}

#[tokio::test(start_paused = true)]
async fn a_tick_cut_off_at_its_maximum_duration_is_retried_so_the_chain_goes_on() {
    let task = FakeTask::new(Behaviour::StuckFirstCall);
    let test = harness(&task, Schedule::every(3600).unwrap()).await;
    let first = start(&test).await;
    // Cut off at 30 min, retried 1 min later.
    test.advance_time_by(time::between(test.now(), first) + 40 * MINUTE).await;
    assert_eq!(task.runs(), vec![first]);
    assert_eq!(state(&test).await.next_run, Some(plus(first, HOUR)));
    assert!(test.failures().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn run_now_does_the_work_at_once_and_replaces_the_armed_tick_so_exactly_one_stays_armed() {
    let task = FakeTask::new(Behaviour::Record);
    let test = harness(&task, Schedule::every(3600).unwrap()).await;
    let first = start(&test).await;
    let armed = test.pending(&job()).await;
    assert_eq!(armed.len(), 1);
    test.advance_time_by(time::between(test.now(), first) - MINUTE).await; // just before the slot
    test.send(&job(), CronMsg::RunNow).await;
    test.run_until_idle().await;
    assert_eq!(task.runs().len(), 1, "ran at once");
    assert_eq!(test.outcome(&armed[0]).await.unwrap().status, OutcomeStatus::Cancelled);
    let next = test.pending(&job()).await;
    assert_eq!(next.len(), 1); // exactly one tick armed
    let s = state(&test).await;
    assert_eq!(s.next_run, Some(first), "the next slot after now is the same slot");
    assert_eq!(s.armed_tick.as_ref(), Some(&next[0].id));

    test.advance_time_by(HOUR).await;
    assert_eq!(task.runs().len(), 2);
    assert_eq!(test.pending(&job()).await.len(), 1);
}
