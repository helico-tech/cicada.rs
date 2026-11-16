//! `cicada-sample-executor worker --server` from the sample image: an authenticated send-only server and the executor
//! on a docker network of their own, both started with `docker run`. The executor gets its entry point overridden, no
//! health check, no database volume, an ambient `CICADA_STORE` it cannot reach and its client token in
//! `CICADA_TOKEN`.

use std::sync::OnceLock;

use cicada::api::MessageState;
use cicada_e2e::cluster::{Node, eventually, run_id};
use cicada_e2e::docker::{IMAGE, SingleNode, docker};
use cicada_e2e::{Class, EXECUTOR_CONTAINERS, EXECUTOR_NETWORK, save_logs, step};
use cicada_examples::registry::DeviceRegistry;
use cicada_sample_service::e2e::{Counter, CounterMsg, CounterState};

use crate::secs;

const TOKEN: &str = "e2e-executor-secret";
/// The executor's worker id, given with `--node-id`; its `CICADA_NODE_ID` names another.
const WORKER_ID: &str = "exec-worker";

pub fn class() -> Class {
    Class {
        before_all: Some(step!(start).1),
        after_all: Some(step!(stop).1),
        ..Class::new(
            "SampleExecutorTest",
            vec![
                step!(the_executor_registers_the_selected_types_without_bootstrap),
                step!(a_counter_runs_under_the_worker_id),
                step!(sigterm_stops_the_executor_with_exit_code_0),
            ],
        )
    }
}

/// The send-only server `exec-server` on host port 9095, requiring `Authorization: Bearer <TOKEN>`.
fn server() -> &'static SingleNode {
    static NODE: OnceLock<SingleNode> = OnceLock::new();
    NODE.get_or_init(|| {
        let env = [("CICADA_NODE_ID", "exec-server"), ("API_TOKEN", TOKEN), ("CICADA_POLL_INTERVAL", "200ms")];
        SingleNode::new(EXECUTOR_CONTAINERS[0], 9095, &env, None, Some(TOKEN))
            .on_network(EXECUTOR_NETWORK, &["--types", "none"])
    })
}

fn node() -> &'static Node {
    &server().node
}

fn executor() -> &'static str {
    EXECUTOR_CONTAINERS[1]
}

async fn start() {
    remove_all().await;
    docker(&["network", "create", EXECUTOR_NETWORK], secs(60), true).await.unwrap();
    server().start().await.unwrap();
    server().await_healthy().await;
    let target = format!("http://{}:8080", server().name);
    let token = format!("CICADA_TOKEN={TOKEN}");
    #[rustfmt::skip]
    let args = [
        "run", "-d", "--name", executor(), "--network", EXECUTOR_NETWORK, "--no-healthcheck",
        "--entrypoint", "/usr/local/bin/cicada-sample-executor",
        "-e", &token,
        "-e", "CICADA_STORE=postgres://cicada:cicada@127.0.0.1:1/unreachable",
        "-e", "CICADA_NODE_ID=exec-env",
        "-e", "CICADA_POLL_INTERVAL=200ms",
        IMAGE,
        "worker", "--server", &target, "--types", "e2e.counter,e2e.slow", "--node-id", WORKER_ID,
    ];
    docker(&args, secs(120), true).await.unwrap();
}

/// Removes both containers with their anonymous volumes, and the network (none of them an error when absent).
async fn remove_all() {
    for container in EXECUTOR_CONTAINERS {
        let _ = docker(&["rm", "-f", "-v", container], secs(60), false).await;
    }
    let _ = docker(&["network", "rm", "-f", EXECUTOR_NETWORK], secs(60), false).await;
}

async fn stop() {
    let executor_logs = docker(&["logs", "--timestamps", executor()], secs(60), false)
        .await
        .map(|o| o.text)
        .unwrap_or_else(|e| e.to_string());
    save_logs("executor", &format!("{}\n----- executor -----\n{executor_logs}", server().logs().await));
    remove_all().await;
}

async fn the_executor_registers_the_selected_types_without_bootstrap() {
    let node = node();
    let names = eventually("the executor registered its types", secs(90), async || {
        let mut names: Vec<String> = node.client.types().await?.into_iter().map(|t| t.name.to_string()).collect();
        names.sort();
        Ok((names.len() == 2).then_some(names))
    })
    .await;
    assert_eq!(names, ["e2e.counter", "e2e.slow"]);
    assert!(node.client.get(&DeviceRegistry).await.unwrap().is_none(), "no singleton");
    let dead_letters = node.get("/actors/cicada.dead-letters/_").await;
    assert_eq!(dead_letters.status, 404, "no dead letters: {dead_letters}");
    let watches = node.get("/watches").await;
    assert_eq!(watches.status, 200, "{watches}");
    assert_eq!(watches.json()["items"], serde_json::json!([]), "no watch: {watches}");
}

async fn a_counter_runs_under_the_worker_id() {
    let node = node();
    let counter = Counter { id: format!("executor-{}", run_id()) };
    node.client.create(&counter, &CounterState::default()).await.unwrap();
    let sent = node.client.send(&counter, CounterMsg::Step { seq: 1, hold_millis: 0 }).await.unwrap();
    assert_eq!(node.await_finished(&sent.id).await.status, MessageState::Done);
    let entries = node.state::<CounterState, _>(&counter).await.entries;
    assert_eq!(entries.iter().map(|e| e.node.as_str()).collect::<Vec<_>>(), [WORKER_ID]);
    assert_eq!(node.get("/").await.text("/node").as_deref(), Some("exec-server"), "the server keeps its id");
}

async fn sigterm_stops_the_executor_with_exit_code_0() {
    docker(&["stop", "-t", "15", executor()], secs(60), true).await.unwrap();
    let state = docker(&["inspect", "-f", "{{.State.ExitCode}} {{.State.OOMKilled}}", executor()], secs(60), true)
        .await
        .unwrap();
    assert_eq!(state.text.trim(), "0 false", "the exit code after SIGTERM");
}
