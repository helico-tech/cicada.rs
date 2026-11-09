//! End-to-end dispatch on PostgreSQL: several nodes (each with its own store and pool) on one schema, real time.

mod common;

use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cicada::prelude::*;
use cicada::runtime::NodeConfig;
use cicada::store::{MessageStatus, NodeId, OutcomeStatus, Store};
use cicada::{SendResult, UriTemplate, time};
use cicada_store_conformance::outcome_record;
use cicada_store_postgres::{PostgresStore, PostgresStoreConfig};
use common::{TestDb, eventually};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Player {
    name: String,
}

impl ActorRef for Player {
    const TEMPLATE: &'static str = "actor://player/{name}";
    type Message = PlayerMsg;
}

fn player(name: &str) -> Player {
    Player { name: name.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum PlayerMsg {
    Ball { hits: u32, until: u32 },
    Tick { left: u32 },
    Slow { millis: u64 },
    Ping,
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
    #[serde(default)]
    pings: u32,
}

impl PlayerState {
    fn new(opponent: &str) -> Self {
        Self { opponent: opponent.into(), hits: 0, ticks: 0, slow: 0, pings: 0 }
    }
}

/// Records whether two turns of one instance ever overlap (G1).
#[derive(Clone, Default)]
struct Overlaps {
    in_turn: Arc<Mutex<HashSet<String>>>,
    seen: Arc<AtomicBool>,
}

struct PlayerActor {
    overlaps: Overlaps,
}

impl PlayerActor {
    async fn guarded(&self, name: &str) {
        if !self.overlaps.in_turn.lock().unwrap().insert(name.to_owned()) {
            self.overlaps.seen.store(true, Ordering::SeqCst);
        }
        tokio::time::sleep(Duration::from_millis(5)).await; // widen the window in which an overlap would be seen
        self.overlaps.in_turn.lock().unwrap().remove(name);
    }
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
                self.guarded(&name).await;
                turn.state_mut().hits += 1;
                if hits < until {
                    let opponent = player(&turn.state().opponent);
                    turn.send(&opponent, PlayerMsg::Ball { hits: hits + 1, until })?;
                }
            }
            PlayerMsg::Tick { left } => {
                self.guarded(&name).await;
                turn.state_mut().ticks += 1;
                if left > 1 {
                    turn.send_self_with(
                        PlayerMsg::Tick { left: left - 1 },
                        SendOptions::new().delay(Duration::from_millis(100)),
                    )?;
                }
            }
            PlayerMsg::Slow { millis } => {
                if turn.message().attempt == 1 {
                    tokio::time::sleep(Duration::from_millis(millis)).await;
                }
                turn.state_mut().slow += 1;
            }
            PlayerMsg::Ping => turn.state_mut().pings += 1,
        }
        Ok(())
    }
}

struct Cluster {
    db: TestDb,
    overlaps: Overlaps,
    systems: Mutex<Vec<ActorSystem>>,
}

impl Cluster {
    fn new(db: TestDb) -> Self {
        Self { db, overlaps: Overlaps::default(), systems: Mutex::new(Vec::new()) }
    }

    async fn store(&self) -> Arc<PostgresStore> {
        self.db.open_with(PostgresStoreConfig { max_pool_size: 6, ..self.db.config() }).await
    }

    async fn node(&self, name: &str, poll: Duration, hosting: bool) -> ActorSystem {
        let store = self.store().await;
        self.node_on(name, poll, hosting, store)
    }

    fn node_on(&self, name: &str, poll: Duration, hosting: bool, store: Arc<PostgresStore>) -> ActorSystem {
        let mut builder = ActorSystem::builder(store).config(NodeConfig {
            node_id: NodeId::new(name).unwrap(),
            lease_ttl: Duration::from_secs(2),
            poll_interval: poll,
            concurrency: 4,
            ..NodeConfig::default()
        });
        if hosting {
            builder = builder.host(PlayerActor { overlaps: self.overlaps.clone() });
        }
        let system = builder.build();
        self.systems.lock().unwrap().push(system.clone());
        system
    }

    async fn cleanup(&self) {
        let systems: Vec<ActorSystem> = std::mem::take(&mut *self.systems.lock().unwrap());
        for system in systems {
            system.stop_with_grace(Duration::ZERO).await;
        }
        self.db.cleanup().await;
    }

    fn overlapped(&self) -> bool {
        self.overlaps.seen.load(Ordering::SeqCst)
    }
}

const POLL: Duration = Duration::from_millis(200);

async fn state(store: &dyn Store, name: &str) -> PlayerState {
    let address = player(name).address().unwrap();
    let record = store.get(&address).await.unwrap().expect("player exists");
    serde_json::from_value(record.state.expect("player has state")).unwrap()
}

fn accepted(result: SendResult) -> cicada::store::Message {
    match result {
        SendResult::Accepted(message) => message,
        other => panic!("expected accepted, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_nodes_play_ping_pong_run_timers_and_recover_stopped_and_crashed_nodes_turns() {
    let cluster = Cluster::new(test_db!("rt"));
    let first = cluster.node("first", POLL, true).await;
    let second = cluster.node("second", POLL, true).await;
    first.start().await.unwrap();
    second.start().await.unwrap();
    let store = first.store().clone();
    let wait = Duration::from_secs(20);

    first.create(&player("ping"), &PlayerState::new("pong")).await.unwrap();
    second.create(&player("pong"), &PlayerState::new("ping")).await.unwrap();
    first.send(&player("ping"), PlayerMsg::Ball { hits: 1, until: 40 }).await.unwrap();
    second.send(&player("pong"), PlayerMsg::Tick { left: 5 }).await.unwrap();
    // A delayed send is not handled before its time.
    let sent_at = store.now().await.unwrap();
    let delayed = accepted(
        first
            .send_with(&player("ping"), PlayerMsg::Ping, SendOptions::new().delay(Duration::from_secs(1)))
            .await
            .unwrap(),
    );
    eventually(wait, "40 hits and 5 ticks", || async {
        state(&*store, "ping").await.hits + state(&*store, "pong").await.hits == 40
            && state(&*store, "pong").await.ticks == 5
    })
    .await;
    assert_eq!(state(&*store, "ping").await.hits, 20);
    assert_eq!(state(&*store, "pong").await.hits, 20);
    eventually(wait, "the delayed ping", || async { outcome_record(&*store, &delayed.id).await.is_some() }).await;
    let finished_at = outcome_record(&*store, &delayed.id).await.unwrap().finished_at;
    assert!(finished_at >= time::add(sent_at, Duration::from_secs(1)).unwrap(), "delayed message handled early");
    assert_eq!(state(&*store, "ping").await.pings, 1);

    // Stop a node mid-turn: its lease is released and another node completes the message (attempt 2).
    second.stop_with_grace(Duration::ZERO).await;
    let third = cluster.node("third", POLL, true).await;
    let slow = accepted(first.send(&player("pong"), PlayerMsg::Slow { millis: 30_000 }).await.unwrap());
    eventually(wait, "the slow turn runs", || async {
        matches!(store.status(&slow.id).await.unwrap(), Some(MessageStatus::Running { .. }))
    })
    .await;
    first.stop_with_grace(Duration::from_millis(100)).await;
    third.start().await.unwrap();
    eventually(wait, "the slow message finishes", || async { outcome_record(&*store, &slow.id).await.is_some() }).await;
    let outcome = outcome_record(&*store, &slow.id).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!(outcome.envelope.attempt, 2);
    assert_eq!(state(&*store, "pong").await.slow, 1);

    // A crashed node never commits nor releases: the turn is re-run after the lease TTL.
    third.stop_with_grace(Duration::ZERO).await;
    let crashed = cluster.store().await;
    let sender = cluster.node("sender", POLL, false).await;
    let lost = accepted(sender.send(&player("ping"), PlayerMsg::Slow { millis: 0 }).await.unwrap());
    let templates = BTreeSet::from([UriTemplate::parse(Player::TEMPLATE).unwrap()]);
    let claim = crashed
        .claim_due(&templates, &NodeId::new("crashed").unwrap(), Duration::from_secs(1), 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(claim.message.id, lost.id);
    crashed.close().await;
    let fourth = cluster.node("fourth", POLL, true).await;
    fourth.start().await.unwrap();
    eventually(wait, "the lost message finishes", || async { outcome_record(&*store, &lost.id).await.is_some() }).await;
    let outcome = outcome_record(&*store, &lost.id).await.unwrap();
    assert_eq!(outcome.envelope.attempt, 2);
    assert!(outcome.finished_at >= claim.lease_until, "re-run only after the lease expired");
    assert!(!cluster.overlapped(), "turns of one instance overlapped");
    cluster.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turns_of_one_instance_stay_serial_under_load_from_several_nodes() {
    let cluster = Cluster::new(test_db!("rt"));
    let mut nodes = Vec::new();
    for i in 0..3 {
        let node = cluster.node(&format!("n{i}"), POLL, true).await;
        node.start().await.unwrap();
        nodes.push(node);
    }
    let names: Vec<String> = (0..4).map(|i| format!("p{i}")).collect();
    for name in &names {
        nodes[0].create(&player(name), &PlayerState::new(name)).await.unwrap();
    }
    // Every node sends to every instance concurrently.
    let sends = nodes.iter().flat_map(|node| {
        names
            .iter()
            .flat_map(move |name| (0..10).map(move |_| async move { node.send(&player(name), PlayerMsg::Ping).await }))
    });
    let ids: Vec<_> = futures::future::join_all(sends).await.into_iter().map(|r| accepted(r.unwrap()).id).collect();
    assert_eq!(ids.len(), 120);
    let store = nodes[0].store().clone();
    eventually(Duration::from_secs(30), "every ping finishes", || async {
        for id in &ids {
            if outcome_record(&*store, id).await.is_none() {
                return false;
            }
        }
        true
    })
    .await;
    for id in &ids {
        assert_eq!(outcome_record(&*store, id).await.unwrap().status, OutcomeStatus::Done);
    }
    for name in &names {
        assert_eq!(state(&*store, name).await.pings, 30);
    }
    assert!(!cluster.overlapped(), "turns of one instance overlapped");
    cluster.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn listen_notify_wakes_a_node_long_before_its_poll_interval() {
    let cluster = Cluster::new(test_db!("rt"));
    let host_store = cluster.store().await;
    let host = cluster.node_on("host", Duration::from_secs(10), true, host_store.clone());
    let sender = cluster.node("sender", POLL, false).await;
    host.start().await.unwrap();
    sender.start().await.unwrap();
    eventually(Duration::from_secs(10), "the host listens", || async { host_store.is_listening() }).await;
    tokio::time::sleep(Duration::from_millis(300)).await; // let the host's dispatch loop settle into its 10 s sleep

    sender.create(&player("far"), &PlayerState::new("far")).await.unwrap();
    let mark = Instant::now();
    let now = accepted(sender.send(&player("far"), PlayerMsg::Ping).await.unwrap());
    eventually(Duration::from_secs(1), "the ping is handled", || async {
        outcome_record(&*host_store, &now.id).await.is_some()
    })
    .await;
    assert!(mark.elapsed() < Duration::from_secs(1));

    // A delayed send wakes the node too, which then sleeps only until the message is due.
    let delayed_mark = Instant::now();
    let delayed = accepted(
        sender
            .send_with(&player("far"), PlayerMsg::Ping, SendOptions::new().delay(Duration::from_millis(1500)))
            .await
            .unwrap(),
    );
    eventually(Duration::from_secs(4), "the delayed ping is handled", || async {
        outcome_record(&*host_store, &delayed.id).await.is_some()
    })
    .await;
    assert!(delayed_mark.elapsed() >= Duration::from_millis(1500));
    assert_eq!(state(&*host_store, "far").await.pings, 2);
    cluster.cleanup().await;
}
