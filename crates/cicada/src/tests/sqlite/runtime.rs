//! End-to-end dispatch on a real backend, two nodes and one SQLite
//! file, real time.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::common::TempDb;
use crate::prelude::*;
use crate::runtime::NodeConfig;
use crate::store::sqlite::{SqliteOptions, SqliteStore};
use crate::store::{MessageStatus, NodeId, OutcomeStatus, Store, SystemClock};
use crate::tests::conformance::outcome_record;
use crate::{SendResult, Turn};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Player {
    name: String,
}

impl ActorRef for Player {
    const TYPE: &'static str = "player";
    type Message = PlayerMsg;
}

fn player(name: &str) -> Player {
    Player { name: name.to_owned() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum PlayerMsg {
    Ball { hits: u32, until: u32 },
    Tick { left: u32 },
    Slow { millis: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct PlayerState {
    opponent: String,
    #[serde(default)]
    hits: u32,
    #[serde(default)]
    ticks: u32,
    #[serde(default)]
    slow: u32,
}

impl PlayerState {
    fn new(opponent: &str) -> Self {
        Self { opponent: opponent.to_owned(), hits: 0, ticks: 0, slow: 0 }
    }
}

/// Records whether two turns of one instance ever overlap (G1).
#[derive(Default)]
struct OverlapGuard {
    in_turn: Mutex<HashMap<String, bool>>,
    overlaps: AtomicBool,
}

impl OverlapGuard {
    async fn guard<T>(&self, name: &str, block: impl Future<Output = T>) -> T {
        {
            let mut in_turn = self.in_turn.lock().unwrap();
            let flag = in_turn.entry(name.to_owned()).or_default();
            if *flag {
                self.overlaps.store(true, Ordering::SeqCst);
            }
            *flag = true;
        }
        let result = block.await;
        self.in_turn.lock().unwrap().insert(name.to_owned(), false);
        result
    }
}

struct PlayerActor {
    guard: Arc<OverlapGuard>,
}

impl Actor for PlayerActor {
    type Ref = Player;
    type State = PlayerState;
    type Message = PlayerMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: PlayerMsg) -> HandlerResult {
        let name = turn.self_ref().name.clone();
        match message {
            PlayerMsg::Ball { hits, until } => {
                self.guard
                    .guard(&name, async {
                        turn.state_mut().hits += 1;
                        if hits < until {
                            let opponent = player(&turn.state().opponent);
                            turn.send(&opponent, PlayerMsg::Ball { hits: hits + 1, until })?;
                        }
                        Ok(())
                    })
                    .await
            }
            PlayerMsg::Tick { left } => {
                self.guard
                    .guard(&name, async {
                        turn.state_mut().ticks += 1;
                        if left > 1 {
                            turn.send_self_with(
                                PlayerMsg::Tick { left: left - 1 },
                                SendOptions::new().delay(Duration::from_millis(100)),
                            )?;
                        }
                        Ok(())
                    })
                    .await
            }
            PlayerMsg::Slow { millis } => {
                if turn.message().attempt == 1 {
                    tokio::time::sleep(Duration::from_millis(millis)).await;
                }
                turn.state_mut().slow += 1;
                Ok(())
            }
        }
    }
}

struct Cluster {
    db: TempDb,
    guard: Arc<OverlapGuard>,
    stores: Vec<Arc<SqliteStore>>,
    systems: Vec<ActorSystem>,
}

impl Cluster {
    async fn node(&mut self, name: &str) -> ActorSystem {
        let options = SqliteOptions { clock: Arc::new(SystemClock), ..SqliteOptions::default() };
        let store = Arc::new(SqliteStore::open(&self.db.file, options).await.expect("the store opens"));
        self.stores.push(store.clone());
        let system = ActorSystem::builder(store)
            .host(PlayerActor { guard: self.guard.clone() })
            .config(NodeConfig {
                node_id: NodeId::new(name).unwrap(),
                lease_ttl: Duration::from_secs(2),
                poll_interval: Duration::from_millis(50),
                concurrency: 4,
                ..NodeConfig::default()
            })
            .build();
        self.systems.push(system.clone());
        system
    }

    async fn clean_up(self) {
        for system in &self.systems {
            system.stop_with_grace(Duration::ZERO).await;
        }
        for store in &self.stores {
            store.close().await;
        }
    }
}

async fn eventually<F: Future<Output = bool>>(mut condition: impl FnMut() -> F) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the condition holds within 20 s");
}

async fn state(store: &Arc<dyn Store>, name: &str) -> PlayerState {
    let record = store.get(&player(name).address().unwrap()).await.unwrap().expect("the player exists");
    serde_json::from_value(record.state).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_nodes_on_one_sqlite_file_play_ping_pong_run_timers_and_recover_a_stopped_node_turn() {
    let mut cluster = Cluster { db: TempDb::new(), guard: Arc::default(), stores: Vec::new(), systems: Vec::new() };
    let first = cluster.node("first").await;
    let second = cluster.node("second").await;
    first.start().await.unwrap();
    second.start().await.unwrap();
    let store = first.store().clone();

    first.create(&player("ping"), &PlayerState::new("pong")).await.unwrap();
    second.create(&player("pong"), &PlayerState::new("ping")).await.unwrap();
    first.send(&player("ping"), PlayerMsg::Ball { hits: 1, until: 40 }).await.unwrap();
    second.send(&player("pong"), PlayerMsg::Tick { left: 5 }).await.unwrap();
    eventually(|| {
        let store = store.clone();
        async move {
            let (ping, pong) = (state(&store, "ping").await, state(&store, "pong").await);
            ping.hits + pong.hits == 40 && pong.ticks == 5
        }
    })
    .await;
    assert_eq!(state(&store, "ping").await.hits, 20);
    assert_eq!(state(&store, "pong").await.hits, 20);

    // A bad payload sent through the raw path (a node that does not host the type) ends as the unhandled policy says.
    let outsider = ActorSystem::builder(store.clone()).build();
    let sent = outsider
        .send_message(&player("ping").address().unwrap(), "Ball", json!({"hits": "x"}), SendOptions::default())
        .await
        .unwrap();
    let SendResult::Accepted(bad) = sent else { panic!("accepted, got {sent:?}") };
    eventually(|| {
        let (store, id) = (store.clone(), bad.id.clone());
        async move { outcome_record(&*store, &id).await.is_some() }
    })
    .await;
    assert_eq!(outcome_record(&*store, &bad.id).await.unwrap().status, OutcomeStatus::Failed);

    // Stop a node mid-turn: its lease is released and another node completes the message (attempt 2).
    second.stop_with_grace(Duration::ZERO).await;
    let third = cluster.node("third").await;
    let sent = first.send(&player("pong"), PlayerMsg::Slow { millis: 30_000 }).await.unwrap();
    let SendResult::Accepted(slow) = sent else { panic!("accepted, got {sent:?}") };
    eventually(|| {
        let (store, id) = (store.clone(), slow.id.clone());
        async move { matches!(store.status(&id).await.unwrap(), Some(MessageStatus::Running { .. })) }
    })
    .await;
    first.stop_with_grace(Duration::from_millis(100)).await;
    third.start().await.unwrap();
    eventually(|| {
        let (store, id) = (store.clone(), slow.id.clone());
        async move { outcome_record(&*store, &id).await.is_some() }
    })
    .await;
    let outcome = outcome_record(&*store, &slow.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!(outcome.envelope.attempt, 2);
    assert_eq!(state(&store, "pong").await.slow, 1);
    assert!(!cluster.guard.overlaps.load(Ordering::SeqCst), "turns of one instance overlapped");
    cluster.clean_up().await;
}
