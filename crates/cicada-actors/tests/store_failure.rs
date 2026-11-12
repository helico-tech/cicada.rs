//! A read of the store that fails during a turn of a ready-made actor. The public API has no way to make one read fail,
//! so the registry of a SQLite store is made unreadable for a turn: its table is renamed underneath the node. Real time:
//! SQLite works on threads of its own, which tokio's paused clock does not wait for.

mod common;

use std::path::Path;

use cicada::prelude::*;
use cicada::store::{Message, MessageStatus, OutcomeStatus, SqliteOptions};
use cicada::{ActorAddress, SendResult, Store};
use cicada_actors::{
    Child, Supervisor, SupervisorActor, SupervisorMsg, SupervisorState, Topic, TopicActor, TopicMsg, TopicState,
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
        let system =
            ActorSystem::builder(store.clone()).host(SupervisorActor).host(TopicActor).host(WorkerActor).build();
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

const HOUR: std::time::Duration = std::time::Duration::from_secs(3600);
