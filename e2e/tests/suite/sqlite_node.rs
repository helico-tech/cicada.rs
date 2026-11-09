//! A single node on the SQLite store (a named volume), plus bearer authentication on
//! another node; both started with `docker run`, outside the compose stack.

use std::sync::OnceLock;
use std::time::Instant;

use cicada::SendOptions;
use cicada::api::MessageState;
use cicada_e2e::cluster::{Node, eventually, run_id};
use cicada_e2e::docker::{SingleNode, docker};
use cicada_e2e::{Class, SINGLE_CONTAINERS, SQLITE_VOLUME, save_logs, step};
use cicada_sample_service::e2e::{Counter, CounterMsg, CounterState, Slow, SlowMsg, SlowState};
use reqwest::Method;

use crate::secs;

const TOKEN: &str = "e2e-secret";
const TIMINGS: [(&str, &str); 3] = [("LEASE_TTL", "5s"), ("POLL_INTERVAL", "200ms"), ("SHUTDOWN_GRACE_PERIOD", "5s")];

pub fn class() -> Class {
    Class {
        before_all: Some(step!(start).1),
        after_all: Some(step!(stop).1),
        ..Class::new(
            "SqliteNodeTest",
            vec![
                step!(health_and_types),
                step!(state_pending_messages_and_an_in_flight_turn_survive_a_container_restart),
                step!(bearer_authentication),
            ],
        )
    }
}

/// The SQLite node `sqlite-1` on host port 9090, with its database on a named volume.
fn sqlite() -> &'static SingleNode {
    static NODE: OnceLock<SingleNode> = OnceLock::new();
    NODE.get_or_init(|| {
        let env: Vec<_> = TIMINGS.into_iter().chain([("NODE_ID", "sqlite-1")]).collect();
        SingleNode::new(SINGLE_CONTAINERS[0], 9090, &env, Some(SQLITE_VOLUME), None)
    })
}

/// The node `auth-1` on host port 9091, which requires `Authorization: Bearer e2e-secret`.
fn secured() -> &'static SingleNode {
    static NODE: OnceLock<SingleNode> = OnceLock::new();
    NODE.get_or_init(|| {
        let env: Vec<_> = TIMINGS.into_iter().chain([("NODE_ID", "auth-1"), ("API_TOKEN", TOKEN)]).collect();
        SingleNode::new(SINGLE_CONTAINERS[1], 9091, &env, None, Some(TOKEN))
    })
}

fn node() -> &'static Node {
    &sqlite().node
}

async fn start() {
    let _ = docker(&["volume", "rm", "-f", SQLITE_VOLUME], secs(60), false).await;
    sqlite().start().await.unwrap();
    secured().start().await.unwrap();
    sqlite().await_healthy().await;
    secured().await_healthy().await;
}

async fn stop() {
    let logs = format!("{}\n----- auth node -----\n{}", sqlite().logs().await, secured().logs().await);
    save_logs("sqlite", &logs);
    sqlite().remove().await;
    secured().remove().await;
    let _ = docker(&["volume", "rm", "-f", SQLITE_VOLUME], secs(60), false).await;
}

fn step_msg(seq: i64) -> CounterMsg {
    CounterMsg::Step { seq, hold_millis: 0 }
}

async fn health_and_types() {
    let node = node();
    let health = node.get("/health").await;
    assert_eq!(health.status, 200, "{health}");
    assert_eq!(health.text("/node").as_deref(), Some("sqlite-1"));
    let types = node.client.types().await.unwrap();
    let slow = types.iter().find(|t| t.name == "e2e.slow").expect("the slow type");
    assert!(slow.messages.contains_key("Sleep"));
    assert!(node.get("/").await.body.contains("sqlite"));
}

async fn state_pending_messages_and_an_in_flight_turn_survive_a_container_restart() {
    let node = node();
    let counter = Counter { id: format!("sqlite-restart-{}", run_id()) };
    node.client.create(&counter, &CounterState::default()).await.unwrap();
    let first = node.client.send(&counter, step_msg(1)).await.unwrap();
    assert_eq!(node.await_finished(&first.id).await.status, MessageState::Done);
    let delayed = node.client.send_with(&counter, step_msg(2), SendOptions::new().delay(secs(12))).await.unwrap();
    let slow = Slow { id: format!("sqlite-restart-{}", run_id()) };
    node.client.create(&slow, &SlowState::default()).await.unwrap();
    let in_flight = node.client.send(&slow, SlowMsg::Sleep { millis: 7_000, label: None }).await.unwrap();
    eventually("slow turn running", secs(30), async || {
        Ok((node.try_status(&in_flight.id).await?.status == MessageState::Running).then_some(()))
    })
    .await;

    let restart = Instant::now();
    docker(&["restart", "-t", "15", sqlite().name], secs(120), true).await.unwrap();
    sqlite().await_healthy().await;
    println!("sqlite: restarted in {:.1?}", restart.elapsed());

    let done = node.await_finished_within(&delayed.id, secs(60)).await;
    assert_eq!(done.status, MessageState::Done);
    assert!(done.finished_at.unwrap() >= delayed.deliver_at.unwrap());
    let entries = node.state::<CounterState, _>(&counter).await.entries;
    assert_eq!(entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(node.await_finished_within(&in_flight.id, secs(60)).await.status, MessageState::Done);
    // Longer than the grace period: released at stop, run again after the restart.
    let runs = node.state::<SlowState, _>(&slow).await.runs;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].attempt, 2);
}

async fn bearer_authentication() {
    let node = &secured().node;
    let id = format!("auth-{}", run_id());
    assert_eq!(node.request_as(Method::GET, "/health", None, None).await.unwrap().status, 401);
    assert_eq!(node.request_as(Method::GET, "/types", None, Some("wrong")).await.unwrap().status, 401);
    assert_eq!(node.get("/health").await.status, 200);
    assert_eq!(node.put(&format!("/actors/e2e.counter/{id}"), r#"{"state":{}}"#).await.status, 201);
    let root = node.request_as(Method::GET, "/", None, None).await.unwrap();
    assert_eq!(root.status, 200, "GET / stays public: {root}");
    let receipt = node.client.send(&Counter { id }, step_msg(1)).await.unwrap();
    let status = node.request_as(Method::GET, &format!("/messages/{}", receipt.id), None, None).await.unwrap();
    assert_eq!(status.status, 401);
}
