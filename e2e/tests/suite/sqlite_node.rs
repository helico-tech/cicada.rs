//! Port of `SqliteNodeTest` (T17): a single node on the SQLite store (a named volume), plus bearer authentication on
//! another node; both started with `docker run`, outside the compose stack.

use std::sync::OnceLock;
use std::time::Instant;

use chrono::{TimeDelta, Utc};
use cicada::SendOptions;
use cicada::api::{MessageState, problem_types};
use cicada::store::MessageId;
use cicada_e2e::cluster::{Node, address, eventually, path, run_id};
use cicada_e2e::docker::{SingleNode, docker};
use cicada_e2e::{Class, SINGLE_CONTAINERS, SQLITE_VOLUME, save_logs, step};
use cicada_sample_service::e2e::{
    Counter, CounterMsg, CounterState, Recorder, RecorderMsg, RecorderState, Slow, SlowMsg, SlowState,
};
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
                step!(create_send_delayed_send_status_and_errors),
                step!(idempotent_send_cancel_and_delete),
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

fn record(label: &str) -> RecorderMsg {
    RecorderMsg::Record { label: label.to_owned(), sleep_millis: 0 }
}

async fn health_and_types() {
    let node = node();
    let health = node.get("/health").await;
    assert_eq!(health.status, 200, "{health}");
    assert_eq!(health.text("/node").as_deref(), Some("sqlite-1"));
    let types = node.client.types().await.unwrap();
    let slow = types.iter().find(|t| t.template == "actor://e2e/slow/{id}").expect("the slow type");
    assert!(slow.messages.contains_key("Sleep"));
    let matched = node.client.type_of(&address(&Slow { id: "abc".to_owned() })).await.unwrap().unwrap();
    assert_eq!(matched.params.get("id"), Some("abc"));
    assert!(node.get("/").await.body.contains("sqlite"));
}

async fn create_send_delayed_send_status_and_errors() {
    let node = node();
    let counter = Counter { id: format!("sqlite-{}", run_id()) };
    assert_eq!(node.put(&path(&counter), r#"{"state":{}}"#).await.status, 201);
    assert_eq!(node.put(&path(&counter), r#"{"state":{}}"#).await.status, 200);

    let sent_at = Utc::now();
    let delayed = node.client.send_with(&counter, step_msg(2), SendOptions::new().delay(secs(2))).await.unwrap();
    let now = node.client.send(&counter, step_msg(1)).await.unwrap();
    assert_eq!(node.status(&delayed.id).await.status, MessageState::Pending);
    assert_eq!(node.await_finished(&now.id).await.status, MessageState::Done);
    let done = node.await_finished(&delayed.id).await;
    let due = delayed.deliver_at.unwrap();
    assert!(done.finished_at.unwrap() >= due);
    assert!(due >= sent_at + TimeDelta::seconds(1));
    let state: CounterState = node.state(&counter).await;
    assert_eq!(state.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2]);
    assert!(state.entries.last().unwrap().claimed_at >= due);
    assert_eq!(node.view(&counter).await.version, 2);

    let mailbox = format!("{}/messages", path(&counter));
    let unknown = node.post(&mailbox, r#"{"type":"Nope","payload":{}}"#).await;
    assert_eq!(unknown.status, 422, "{unknown}");
    assert_eq!(unknown.text("/type").as_deref(), Some(problem_types::UNKNOWN_MESSAGE_TYPE));
    let invalid = node.post(&mailbox, r#"{"type":"Step","payload":{"seq":"x"}}"#).await;
    assert_eq!(invalid.status, 422, "{invalid}");
    assert_eq!(invalid.text("/type").as_deref(), Some(problem_types::INVALID_PAYLOAD));
    let absent = format!("/actors/e2e/counter/absent-{}/messages", run_id());
    assert_eq!(node.post(&absent, r#"{"type":"Step","payload":{"seq":1}}"#).await.status, 404);
}

async fn idempotent_send_cancel_and_delete() {
    let node = node();
    let recorder = Recorder { id: format!("sqlite-{}", run_id()) };
    let path = path(&recorder);
    node.client.create(&recorder, &RecorderState::default()).await.unwrap();
    let body = format!(r#"{{"id":"{}","type":"Record","payload":{{"label":"once"}}}}"#, MessageId::random());
    assert_eq!(node.post(&format!("{path}/messages"), &body).await.status, 202);
    assert_eq!(node.post(&format!("{path}/messages"), &body).await.status, 200);

    let later = node.client.send_with(&recorder, record("later"), SendOptions::new().delay(secs(600))).await.unwrap();
    assert_eq!(node.delete(&format!("/messages/{}", later.id)).await.status, 204);
    assert_eq!(node.delete(&format!("/messages/{}", later.id)).await.status, 409);
    assert_eq!(node.status(&later.id).await.status, MessageState::Cancelled);

    let purged = node.client.send_with(&recorder, record("purged"), SendOptions::new().delay(secs(600))).await.unwrap();
    eventually("first record", secs(30), async || {
        let state: RecorderState = node.try_state(&recorder).await?;
        Ok((state.records.len() == 1).then_some(()))
    })
    .await;
    assert_eq!(node.delete(&path).await.status, 204);
    assert_eq!(node.status(&purged.id).await.status, MessageState::Purged);
    assert_eq!(node.get(&path).await.status, 404);
    assert_eq!(node.delete(&path).await.status, 404);
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
    assert_eq!(node.put(&format!("/actors/e2e/counter/{id}"), r#"{"state":{}}"#).await.status, 201);
    let root = node.request_as(Method::GET, "/", None, None).await.unwrap();
    assert_eq!(root.status, 200, "GET / stays public: {root}");
    let receipt = node.client.send(&Counter { id }, step_msg(1)).await.unwrap();
    let status = node.request_as(Method::GET, &format!("/messages/{}", receipt.id), None, None).await.unwrap();
    assert_eq!(status.status, 401);
}
