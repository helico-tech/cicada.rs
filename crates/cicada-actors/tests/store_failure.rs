//! A read of the store that fails during a turn of a ready-made actor. The public API has no way to make one read fail,
//! so the registry of a SQLite store is made unreadable for a turn: its table is renamed underneath the node. Real time:
//! SQLite works on threads of its own, which tokio's paused clock does not wait for.

mod common;

use std::path::Path;

use cicada::prelude::*;
use cicada::store::{Message, MessageStatus, OutcomeStatus, SqliteOptions};
use cicada::{ActorAddress, FailureCause, SendResult, Store, WatchTarget};
use cicada_actors::{
    Batcher, BatcherActor, BatcherMsg, BatcherState, Child, Debouncer, DebouncerActor, DebouncerMsg, DebouncerState,
    Event, EventActor, EventMsg, EventState, Flow, FlowActor, FlowRun, FlowState, FlowStatus, RateLimiter,
    RateLimiterActor, RateLimiterMsg, RateLimiterState, ReplyTo, Schedule, Scheduler, SchedulerActor, SchedulerMsg,
    SchedulerState, Step, Supervisor, SupervisorActor, SupervisorMsg, SupervisorState, Target, Topic, TopicActor,
    TopicMsg, TopicState,
};
use common::*;
use sqlx::SqlitePool;
use tempfile::TempDir;

/// A node on a SQLite store in `dir` that hosts the ready-made actors and the worker, and a second connection to the
/// database, to take the registry away.
struct Sqlite {
    system: ActorSystem,
    store: Store,
    pool: SqlitePool,
}

impl Sqlite {
    async fn open(dir: &Path) -> Self {
        let path = dir.join("cicada.db");
        let store = Store::sqlite(&path, SqliteOptions::default()).await.unwrap();
        let system = ActorSystem::builder(store.clone())
            .host(SupervisorActor)
            .host(TopicActor)
            .host(SchedulerActor)
            .host(RateLimiterActor)
            .host(DebouncerActor)
            .host(BatcherActor)
            .host(EventActor)
            .host(FlowActor::new(Mail))
            .host(FlowActor::new(Ship))
            .host(FlowActor::new(Order))
            .host(WorkerActor)
            .host(WitnessActor)
            .build();
        system.initialize().await.unwrap();
        let pool = SqlitePool::connect(&format!("sqlite://{}", path.display())).await.unwrap();
        Self { system, store, pool }
    }

    /// Makes every read of the registry fail, or work again.
    async fn registry(&self, readable: bool) {
        let sql = if readable {
            "ALTER TABLE actor_types_away RENAME TO actor_types"
        } else {
            "ALTER TABLE actor_types RENAME TO actor_types_away"
        };
        sqlx::query(sql).execute(&self.pool).await.unwrap();
    }

    async fn run_until_idle(&self) {
        while self.system.run_due_turns(10).await.unwrap() > 0 {}
    }

    async fn pending(&self, address: &ActorAddress) -> Vec<Message> {
        self.system.pending(address, None, 100).await.unwrap().map(|page| page.items).unwrap_or_default()
    }

    async fn outcome(&self, message: &Message) -> (OutcomeStatus, String) {
        match self.system.status(&message.id).await.unwrap() {
            Some(MessageStatus::Finished(o)) => (o.status, o.error.unwrap_or_default()),
            other => panic!("{} has no outcome: {other:?}", message.id),
        }
    }

    async fn state<S: serde::de::DeserializeOwned>(&self, address: &ActorAddress) -> S {
        let snapshot = self.system.get_actor(address).await.unwrap().expect("the actor exists");
        serde_json::from_value(snapshot.state).unwrap()
    }

    async fn close(self) {
        self.pool.close().await;
        self.store.close().await;
    }
}

/// Scenario: The store fails during a restart.
#[tokio::test]
async fn a_read_of_the_store_that_fails_during_a_restart_commits_nothing_and_the_child_keeps_running() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    let sup = Supervisor::named("s");
    let state = SupervisorState::new().child(Child::new(&worker("a"), &WorkerState::default()).unwrap());
    db.system.create(&sup, &state).await.unwrap();
    db.system.send(&sup, SupervisorMsg::Start).await.unwrap();
    db.run_until_idle().await;

    db.system.pause_mailbox(&addr(&sup)).await.unwrap();
    db.system.send(&worker("a"), WorkerMsg::Work).await.unwrap();
    let pending = db.system.send_with(&worker("a"), WorkerMsg::Work, SendOptions::new().delay(HOUR)).await.unwrap();
    db.system.send(&worker("a"), WorkerMsg::Fail).await.unwrap();
    db.run_until_idle().await;
    let failed = db.pending(&addr(&sup)).await;
    assert_eq!(failed.len(), 1, "the Failed of the child waits");

    db.registry(false).await;
    db.system.resume_mailbox(&addr(&sup)).await.unwrap();
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1);
    db.registry(true).await;

    let (status, error) = db.outcome(&failed[0]).await;
    assert_eq!(status, OutcomeStatus::Failed, "the Failed ends failed: a failed read of the store, the cause node");
    assert!(error.contains("actor_types"), "{error}");
    let held: SupervisorState = db.state(&addr(&sup)).await;
    let entry = &held.children[&addr(&worker("a"))];
    assert!(entry.restarts.is_empty() && entry.restart_id.is_none() && entry.running, "nothing committed: {entry:?}");
    assert_eq!(db.state::<WorkerState>(&addr(&worker("a"))).await.n, 1, "the child keeps running");
    let SendResult::Accepted(pending) = pending else { panic!("{pending:?}") };
    assert!(matches!(db.system.status(&pending.id).await.unwrap(), Some(MessageStatus::Pending(_))), "its mailbox");
    db.close().await;
}

/// Scenario: A delivery that is tried again.
#[tokio::test]
async fn a_delivery_whose_read_of_the_store_fails_is_tried_again_and_arrives_once() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    let topic = Topic::named("orders");
    db.system.create(&topic, &TopicState::default()).await.unwrap();
    db.system.create(&worker("a"), &WorkerState::default()).await.unwrap();
    db.system.send(&topic, TopicMsg::Subscribe { subscriber: addr(&worker("a")) }).await.unwrap();
    db.run_until_idle().await;
    let publish = TopicMsg::publish(WorkerMsg::Note { text: "shipped".into() }).unwrap();
    db.system.send(&topic, publish).await.unwrap();
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the publish");
    let deliver = db.pending(&addr(&topic)).await;
    assert_eq!(deliver.len(), 1);

    db.registry(false).await;
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the first attempt of the delivery");
    db.registry(true).await;
    let retried = db.pending(&addr(&topic)).await;
    assert_eq!((retried.len(), retried[0].id.clone(), retried[0].failures), (1, deliver[0].id.clone(), 1));
    assert!(retried[0].last_error.as_deref().unwrap().contains("actor_types"), "{:?}", retried[0].last_error);
    assert!(db.state::<WorkerState>(&addr(&worker("a"))).await.log.is_empty(), "nothing was sent");

    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    db.run_until_idle().await;
    assert_eq!(db.state::<WorkerState>(&addr(&worker("a"))).await.log, ["shipped"], "once, from a later attempt");
    assert_eq!(db.outcome(&deliver[0]).await.0, OutcomeStatus::Done);
    db.close().await;
}

/// Scenario: A run whose read of the store fails is retried. The armed tick of a daily schedule is made due now.
#[tokio::test]
async fn a_run_of_a_scheduler_whose_read_of_the_store_fails_is_tried_again_and_delivers_once() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    let nightly = Scheduler::named("nightly");
    let state = SchedulerState::new(Schedule::daily(2, 0).unwrap(), &worker("a"), WorkerMsg::Note { text: "n".into() })
        .unwrap();
    db.system.create(&nightly, &state).await.unwrap();
    db.system.create(&worker("a"), &WorkerState::default()).await.unwrap();
    db.system.send(&nightly, SchedulerMsg::Start).await.unwrap();
    db.run_until_idle().await;
    let tick = db.pending(&addr(&nightly)).await.pop().expect("the armed tick");
    db.system.deliver_now(&tick.id).await.unwrap();
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the tick");
    let run = db.pending(&addr(&nightly)).await.into_iter().find(|m| m.message_type == "Run").expect("the run");

    db.registry(false).await;
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the first attempt of the run");
    db.registry(true).await;
    let retried = db.pending(&addr(&nightly)).await.into_iter().find(|m| m.id == run.id).expect("pending again");
    assert_eq!(retried.failures, 1);
    assert!(retried.last_error.as_deref().unwrap().contains("actor_types"), "{:?}", retried.last_error);
    assert!(db.state::<WorkerState>(&addr(&worker("a"))).await.log.is_empty(), "nothing was sent");

    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    db.run_until_idle().await;
    assert_eq!(db.state::<WorkerState>(&addr(&worker("a"))).await.log, ["n"], "once, from a later attempt");
    assert_eq!(db.outcome(&run).await.0, OutcomeStatus::Done);
    assert_eq!(db.state::<SchedulerState>(&addr(&nightly)).await.runs, 1);
    db.close().await;
}

/// Scenario: Temporary store failure. A submission with a retry policy whose read of the registry fails reserves
/// nothing; its retry forwards once, at a slot of the store time of that attempt.
#[tokio::test]
async fn a_submission_to_a_rate_limiter_whose_read_of_the_store_fails_is_tried_again_and_forwards_once() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    let api = RateLimiter::named("api");
    let second = std::time::Duration::from_secs(1);
    db.system.create(&api, &RateLimiterState::new(1, second, 1, second).unwrap()).await.unwrap();
    db.system.create(&worker("a"), &WorkerState::default()).await.unwrap();
    let submit = RateLimiterMsg::submit(&worker("a"), WorkerMsg::Note { text: "n".into() }, None).unwrap();
    let policy = RetryPolicy::new(2, Backoff::fixed(second).unwrap()).unwrap();
    let SendResult::Accepted(submission) =
        db.system.send_with(&api, submit, SendOptions::new().retry(policy)).await.unwrap()
    else {
        panic!("not accepted")
    };

    db.registry(false).await;
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the first attempt of the submission");
    db.registry(true).await;
    let retried = db.pending(&addr(&api)).await;
    assert_eq!((retried.len(), retried[0].id.clone(), retried[0].failures), (1, submission.id.clone(), 1));
    assert!(retried[0].last_error.as_deref().unwrap().contains("actor_types"), "{:?}", retried[0].last_error);
    assert_eq!(db.state::<RateLimiterState>(&addr(&api)).await.idle_at, None, "nothing was reserved");
    assert!(db.pending(&addr(&worker("a"))).await.is_empty(), "nothing was sent");

    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    db.run_until_idle().await;
    assert_eq!(db.state::<WorkerState>(&addr(&worker("a"))).await.log, ["n"], "once, from a later attempt");
    assert_eq!(db.outcome(&submission).await.0, OutcomeStatus::Done);
    let idle_at = db.state::<RateLimiterState>(&addr(&api)).await.idle_at.expect("a reservation");
    assert!(idle_at > submission.deliver_at + second, "a slot of the retry's store time: {idle_at}");
    db.close().await;
}

/// Scenario: Temporary store error. The armed flush of a debouncer is made due now; its read of the registry fails,
/// which clears nothing, and its retry forwards the kept value once.
#[tokio::test]
async fn a_flush_of_a_debouncer_whose_read_of_the_store_fails_is_tried_again_and_forwards_once() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    let (deb, timer) = armed_debouncer(&db).await;

    db.registry(false).await;
    db.system.deliver_now(&timer.id).await.unwrap();
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the first attempt of the flush");
    db.registry(true).await;
    let retried = db.pending(&addr(&deb)).await;
    assert_eq!((retried.len(), retried[0].id.clone(), retried[0].failures), (1, timer.id.clone(), 1));
    assert!(retried[0].last_error.as_deref().unwrap().contains("actor_types"), "{:?}", retried[0].last_error);
    let held: DebouncerState = db.state(&addr(&deb)).await;
    assert!(held.pending.is_some() && held.armed == Some(timer.id.clone()), "nothing cleared: {held:?}");
    assert!(db.pending(&addr(&worker("a"))).await.is_empty(), "nothing was sent");

    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    db.run_until_idle().await;
    assert_eq!(db.state::<WorkerState>(&addr(&worker("a"))).await.log, ["n"], "once, from a later attempt");
    assert_eq!(db.outcome(&timer).await.0, OutcomeStatus::Done);
    let held: DebouncerState = db.state(&addr(&deb)).await;
    assert!(held.pending.is_none() && held.first_at.is_none() && held.armed.is_none(), "cleared: {held:?}");
    db.close().await;
}

/// Scenario: Replacement cancels a retrying timer.
#[tokio::test]
async fn a_submit_cancels_a_flush_that_waits_for_its_retry_and_arms_its_own() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    let (deb, timer) = armed_debouncer(&db).await;
    db.registry(false).await;
    db.system.deliver_now(&timer.id).await.unwrap();
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the first attempt of the flush");
    db.registry(true).await;

    let submit = DebouncerMsg::submit(&worker("a"), WorkerMsg::Note { text: "m".into() }, None).unwrap();
    db.system.send(&deb, submit).await.unwrap();
    db.run_until_idle().await;
    assert_eq!(db.outcome(&timer).await.0, OutcomeStatus::Cancelled, "the retry is cancelled");
    let pending = db.pending(&addr(&deb)).await;
    let held: DebouncerState = db.state(&addr(&deb)).await;
    assert_eq!(pending.len(), 1, "only the replacement timer");
    assert_eq!(held.armed, Some(pending[0].id.clone()));
    assert_eq!(held.pending.unwrap().message.payload["text"], "m");
    db.close().await;
}

/// Scenario: Temporary registry failure. The armed flush of a batcher is made due now; its read of the registry fails,
/// which clears nothing, and its retry forwards the batch once.
#[tokio::test]
async fn a_flush_of_a_batcher_whose_read_of_the_store_fails_is_tried_again_and_forwards_once() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    let bat = Batcher::named("b");
    db.system.create(&bat, &BatcherState::new(&worker("a"), "Items", 3, HOUR).unwrap()).await.unwrap();
    db.system.create(&worker("a"), &WorkerState::default()).await.unwrap();
    db.system.send(&bat, BatcherMsg::add("n").unwrap()).await.unwrap();
    db.run_until_idle().await;
    let timer = db.pending(&addr(&bat)).await.pop().expect("the armed flush");

    db.registry(false).await;
    db.system.deliver_now(&timer.id).await.unwrap();
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the first attempt of the flush");
    db.registry(true).await;
    let retried = db.pending(&addr(&bat)).await;
    assert_eq!((retried.len(), retried[0].id.clone(), retried[0].failures), (1, timer.id.clone(), 1));
    assert!(retried[0].last_error.as_deref().unwrap().contains("actor_types"), "{:?}", retried[0].last_error);
    let held: BatcherState = db.state(&addr(&bat)).await;
    assert!(held.items.len() == 1 && held.armed == Some(timer.id.clone()), "nothing cleared: {held:?}");
    assert!(db.pending(&addr(&worker("a"))).await.is_empty(), "nothing was sent");

    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    db.run_until_idle().await;
    assert_eq!(db.state::<WorkerState>(&addr(&worker("a"))).await.log, [r#"["n"]"#], "once, from a later attempt");
    assert_eq!(db.outcome(&timer).await.0, OutcomeStatus::Done);
    let held: BatcherState = db.state(&addr(&bat)).await;
    assert!(held.items.is_empty() && held.first_at.is_none() && held.armed.is_none(), "cleared: {held:?}");
    db.close().await;
}

/// `shop.mail`: one step.
struct Mail;

impl Flow for Mail {
    const TYPE: &'static str = "shop.mail";
    type Input = serde_json::Value;

    async fn run(&self, step: &mut Step, _input: serde_json::Value) -> HandlerResult {
        step.run("send", |_key| async { Ok(()) }).await
    }
}

/// `shop.ship`: one step.
struct Ship;

impl Flow for Ship {
    const TYPE: &'static str = "shop.ship";
    type Input = serde_json::Value;

    async fn run(&self, step: &mut Step, _input: serde_json::Value) -> HandlerResult {
        step.run("ship", |_key| async { Ok(()) }).await
    }
}

/// Scenario: Store outage. A publication with a retry policy whose read of the registry fails creates and starts
/// nothing; its retry starts each subscribed run once, under the same id.
#[tokio::test]
async fn a_publication_whose_read_of_the_store_fails_starts_no_run_and_its_retry_starts_each_once() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    let event = Event::named("placed");
    db.system.create(&event, &EventState::default()).await.unwrap();
    for flow in ["shop.mail", "shop.ship"] {
        db.system.send(&event, EventMsg::Subscribe { flow: flow.into() }).await.unwrap();
    }
    db.run_until_idle().await;
    let second = std::time::Duration::from_secs(1);
    let policy = RetryPolicy::new(2, Backoff::fixed(second).unwrap()).unwrap();
    let publish = EventMsg::publish(serde_json::json!({"order": 7})).unwrap();
    let SendResult::Accepted(publication) =
        db.system.send_with(&event, publish, SendOptions::new().retry(policy)).await.unwrap()
    else {
        panic!("not accepted")
    };
    let mail = addr(&FlowRun::<Mail>::named(publication.id.as_str()));
    let ship = addr(&FlowRun::<Ship>::named(publication.id.as_str()));

    db.registry(false).await;
    assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the first attempt of the publication");
    db.registry(true).await;
    let retried = db.pending(&addr(&event)).await;
    assert_eq!((retried.len(), retried[0].id.clone(), retried[0].failures), (1, publication.id.clone(), 1));
    assert!(retried[0].last_error.as_deref().unwrap().contains("actor_types"), "{:?}", retried[0].last_error);
    for run in [&mail, &ship] {
        assert!(db.system.get_actor(run).await.unwrap().is_none(), "no partial run: {run}");
    }

    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    db.run_until_idle().await;
    assert_eq!(db.outcome(&publication).await.0, OutcomeStatus::Done);
    for run in [&mail, &ship] {
        let state: FlowState<serde_json::Value> = db.state(run).await;
        assert_eq!((state.status, state.journal.len()), (FlowStatus::Done, 1), "started once: {run}");
    }
    db.close().await;
}

/// `shop.order`: creates `worker/a` and sends it `Work`, with two attempts a second apart; with the input `"request"`,
/// creates `worker/b` and sends it `Work` as a request, whose route the worker does not use.
struct Order;

impl Flow for Order {
    const TYPE: &'static str = "shop.order";
    type Input = serde_json::Value;

    fn retry() -> RetryPolicy {
        RetryPolicy::new(2, Backoff::fixed(std::time::Duration::from_secs(1)).unwrap()).unwrap()
    }

    async fn run(&self, step: &mut Step, input: serde_json::Value) -> HandlerResult {
        if input == "request" {
            let (to, state) = (worker("b"), WorkerState::default());
            step.request("go", Target::create(&to, &state), |_: ReplyTo<()>| WorkerMsg::Work).await?;
            return Ok(());
        }
        step.send("go", Target::create(&worker("a"), &WorkerState::default()), WorkerMsg::Work).await
    }
}

/// Scenario: Registry failure and invalid target, the failed read; and Rejected request commit, of a request. A send or
/// a request whose check of the registry fails is an ordinary failure of the cause `node`, retried by the flow's
/// policy, with no create, message or entry; a public `Continue` recovers the run, which then sends once.
#[tokio::test]
async fn a_flow_send_whose_read_of_the_registry_fails_is_retried_and_commits_nothing() {
    let dir = TempDir::new().unwrap();
    let db = Sqlite::open(dir.path()).await;
    db.system.create(&Witness, &Told::default()).await.unwrap();
    db.system.watch(&WatchTarget::parse("shop.order").unwrap(), &addr(&Witness)).await.unwrap();
    for (told_before, (key, input, to, kind)) in
        [("o1", "send", "a", "send"), ("o2", "request", "b", "request")].into_iter().enumerate()
    {
        let run = FlowRun::<Order>::named(key);
        db.system.create(&run, &FlowState::new(serde_json::json!(input))).await.unwrap();
        db.system.send(&run, cicada_actors::FlowMsg::Continue).await.unwrap();
        assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the poke arms the continuation");

        db.registry(false).await;
        assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the first attempt");
        let retried = db.pending(&addr(&run)).await;
        assert_eq!(retried.len(), 1);
        let error = retried[0].last_error.clone().unwrap();
        let why = format!("{kind} `go` to worker/{to} could not be checked");
        assert!(error.contains(&why) && error.contains("actor_types"), "{error}");
        assert_eq!(retried[0].failures, 1, "an ordinary failure, retried by the flow's policy");
        tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
        assert_eq!(db.system.run_due_turns(1).await.unwrap(), 1, "the second and last attempt");
        db.registry(true).await;
        db.run_until_idle().await;
        let told: Told = db.state(&addr(&Witness)).await;
        assert_eq!(told.failed.len(), told_before + 1);
        let failed = told.failed.last().unwrap();
        assert_eq!((failed.cause, failed.attempts), (FailureCause::Node, 2));
        let held: FlowState<serde_json::Value> = db.state(&addr(&run)).await;
        assert!(held.journal.is_empty(), "{kind}: no entry");
        assert!(db.system.get_actor(&addr(&worker(to))).await.unwrap().is_none(), "{kind}: no create");

        db.system.send(&run, cicada_actors::FlowMsg::Continue).await.unwrap();
        db.run_until_idle().await;
        let held: FlowState<serde_json::Value> = db.state(&addr(&run)).await;
        assert_eq!((held.status, held.journal.len()), (FlowStatus::Done, 1));
        assert_eq!(held.journal[0].id(), "go");
        assert_eq!(db.state::<WorkerState>(&addr(&worker(to))).await.n, 1, "{kind}: sent once");
    }
    db.close().await;
}

/// The debouncer `d`, an hour quiet, keeping a note for `worker/a`, and its armed flush.
async fn armed_debouncer(db: &Sqlite) -> (Debouncer, Message) {
    let deb = Debouncer::named("d");
    db.system.create(&deb, &DebouncerState::new(HOUR, HOUR).unwrap()).await.unwrap();
    db.system.create(&worker("a"), &WorkerState::default()).await.unwrap();
    let submit = DebouncerMsg::submit(&worker("a"), WorkerMsg::Note { text: "n".into() }, None).unwrap();
    db.system.send(&deb, submit).await.unwrap();
    db.run_until_idle().await;
    let timer = db.pending(&addr(&deb)).await.pop().expect("the armed flush");
    (deb, timer)
}

const HOUR: std::time::Duration = std::time::Duration::from_secs(3600);
