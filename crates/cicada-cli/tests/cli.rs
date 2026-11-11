//! The `cicada` commands against a real `actor_api()` server on a local port, in real time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use cicada::prelude::*;
use cicada::runtime::{ActorSystemBuilder, NodeConfig};
use cicada_axum::actor_api;
use cicada_cli::cli::Cli;
use cicada_cli::config::Env;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Account {
    tenant: String,
    id: i32,
}

impl ActorRef for Account {
    const TYPE: &'static str = "account";
    type Message = AccountMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum AccountMsg {
    Deposit {
        amount: i64,
    },
    Explode,
    /// Never finishes; only a cancel ends its turn.
    Hold,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
struct AccountState {
    #[serde(default)]
    balance: i64,
}

struct AccountActor;

impl Actor for AccountActor {
    type Ref = Account;
    type State = AccountState;
    type Message = AccountMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: AccountMsg) -> HandlerResult {
        match message {
            AccountMsg::Deposit { amount } => turn.state_mut().balance += amount,
            AccountMsg::Explode => return Err(HandlerError::msg("boom")),
            AccountMsg::Hold => std::future::pending::<()>().await,
        }
        Ok(())
    }
}

/// type `maintenance.cleanup`, key `_`: a singleton that keeps no data.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cleanup;

impl ActorRef for Cleanup {
    const TYPE: &'static str = "maintenance.cleanup";
    type Message = CleanupMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CleanupMsg {
    Run,
}

struct CleanupActor;

impl Actor for CleanupActor {
    type Ref = Cleanup;
    type State = NoState;
    type Message = CleanupMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, _: &mut Turn<Self>, _: CleanupMsg) -> HandlerResult {
        Ok(())
    }
}

/// A started node hosting `account`, served on a random local port; its base URL.
async fn server_with(wrap: impl FnOnce(Router) -> Router) -> String {
    server_hosting(|node| node.host(AccountActor), wrap).await
}

/// A started node hosting what `host` adds, served on a random local port behind `wrap`; its base URL.
async fn server_hosting(
    host: impl FnOnce(ActorSystemBuilder) -> ActorSystemBuilder,
    wrap: impl FnOnce(Router) -> Router,
) -> String {
    let store = cicada::Store::memory();
    // A short heartbeat, so a cancelled turn ends within the wait of `eventually_status`.
    let config = NodeConfig {
        poll_interval: Duration::from_millis(50),
        heartbeat_interval: Some(Duration::from_millis(200)),
        ..NodeConfig::default()
    };
    let system = host(ActorSystem::builder(store).config(config)).build();
    system.start().await.expect("the node starts");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().unwrap();
    let app = wrap(actor_api(system));
    tokio::spawn(async move { axum::serve(listener, app).await.expect("the server runs") });
    format!("http://{address}")
}

async fn server() -> String {
    server_with(|app| app).await
}

/// An environment whose config file does not exist, so the user's own config never leaks into a test.
fn no_config() -> Env {
    Env::from([("CICADA_CONFIG".to_owned(), "/nonexistent/cicada/config.toml".to_owned())])
}

/// Runs `cicada <args>` with `env`; its output, or the error as text.
async fn cicada_in(env: &Env, args: &[&str]) -> Result<String, String> {
    let cli = Cli::try_parse_from(std::iter::once("cicada").chain(args.iter().copied())).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    cicada_cli::run(cli, env, &mut out).await.map_err(|e| cicada_cli::error_text(&e))?;
    Ok(String::from_utf8(out).unwrap())
}

/// Runs `cicada --server <server> <args>` and expects it to succeed.
async fn cicada(server: &str, args: &[&str]) -> String {
    let mut all = vec!["--server", server];
    all.extend(args);
    cicada_in(&no_config(), &all).await.unwrap_or_else(|e| panic!("cicada {args:?} failed: {e}"))
}

/// Runs `cicada --server <server> <args>` and expects it to fail; the error.
async fn cicada_fails(server: &str, args: &[&str]) -> String {
    let mut all = vec!["--server", server];
    all.extend(args);
    cicada_in(&no_config(), &all).await.expect_err("the command fails")
}

async fn json(server: &str, args: &[&str]) -> Value {
    let mut all = args.to_vec();
    all.extend(["-o", "json"]);
    serde_json::from_str(&cicada(server, &all).await).unwrap()
}

/// The message id printed by `send` or `retry`.
fn sent_id(output: &str) -> String {
    let value: Value = serde_json::from_str(output).unwrap();
    value["id"].as_str().unwrap().to_owned()
}

async fn eventually_status(server: &str, id: &str, status: &str) {
    for _ in 0..100 {
        if json(server, &["get", "message", id]).await["status"] == status {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("message {id} never reached {status}");
}

/// Data lines of a table (without the header).
fn rows(table: &str) -> Vec<&str> {
    table.lines().skip(1).collect()
}

#[tokio::test]
async fn create_send_and_read_back() {
    let s = server().await;
    let created = cicada(&s, &["create", "actor", "account/acme:7", "--state", r#"{"balance":1}"#]).await;
    assert_eq!(created, "actor account/acme:7 created\n");
    assert!(cicada(&s, &["create", "actor", "account/acme:7", "--state", "{}"]).await.contains("left untouched"));

    let sent =
        cicada(&s, &["send", "account/acme:7", "--type", "Deposit", "--payload", r#"{"amount":5}"#, "-o", "json"]);
    let id = sent_id(&sent.await);
    eventually_status(&s, &id, "done").await;

    let actor = json(&s, &["get", "actor", "account/acme:7"]).await;
    assert_eq!(actor["state"]["balance"], 6);
    assert_eq!(actor["address"], "account/acme:7");

    let table = cicada(&s, &["get", "actor", "account/acme:7"]).await;
    assert!(table.starts_with("ADDRESS") && table.contains("account/acme:7"), "{table}");
    let wide = cicada(&s, &["get", "actor", "account/acme:7", "-o", "wide"]).await;
    assert!(wide.contains("STATE") && wide.contains(r#"{"balance":6}"#), "{wide}");

    let yaml = cicada(&s, &["describe", "actor", "account/acme:7"]).await;
    assert!(yaml.contains("address: account/acme:7") && yaml.contains("balance: 6"), "{yaml}");
    assert!(cicada(&s, &["describe", "message", &id]).await.contains("status: done"));
    assert!(cicada(&s, &["get", "message", &id, "-o", "yaml"]).await.contains("status: done"));

    assert_eq!(rows(&cicada(&s, &["get", "actors"]).await), ["account/acme:7  account  acme:7"]);
    let stats = json(&s, &["stats", "--prefix", "account/"]).await;
    assert_eq!((stats["actors"].as_u64(), stats["pending"].as_u64()), (Some(1), Some(0)));
    assert!(rows(&cicada(&s, &["get", "caused", &id]).await).is_empty());

    let plain =
        cicada(&s, &["send", "account/acme:7", "--type", "Deposit", "--payload", r#"{"amount":1}"#, "--id", "fixed"]);
    assert!(plain.await.starts_with("message fixed sent, due "));
    let again =
        cicada(&s, &["send", "account/acme:7", "--type", "Deposit", "--payload", r#"{"amount":1}"#, "--id", "fixed"]);
    assert_eq!(again.await, "message fixed already accepted\n");

    let at = "2999-01-01T00:00:00Z";
    let args = ["--id", "later", "--at", at, "--header", "k=v", "--correlation-id", "c1"];
    let send = ["send", "account/acme:7", "--type", "Deposit", "--payload", r#"{"amount":1}"#];
    let sent = cicada(&s, &[&send[..], &args].concat()).await;
    assert_eq!(sent, format!("message later sent, due {at}\n"));
    let later = json(&s, &["get", "message", "later"]).await;
    assert_eq!((later["message"]["deliverAt"].as_str(), later["status"].as_str()), (Some(at), Some("pending")));
    assert_eq!(
        (later["message"]["headers"]["k"].as_str(), later["message"]["correlationId"].as_str()),
        (Some("v"), Some("c1"))
    );
}

#[tokio::test]
async fn pending_messages_can_be_listed_moved_cancelled_and_purged() {
    let s = server().await;
    cicada(&s, &["create", "actor", "account/acme:1", "--state", "{}"]).await;
    let later = |id: &'static str| {
        let s = s.clone();
        async move {
            cicada(
                &s,
                &[
                    "send",
                    "account/acme:1",
                    "--type",
                    "Deposit",
                    "--payload",
                    r#"{"amount":1}"#,
                    "--delay",
                    "PT1H",
                    "--id",
                    id,
                ],
            )
            .await
        }
    };
    later("m1").await;
    later("m2").await;
    later("m3").await;

    assert_eq!(rows(&cicada(&s, &["get", "messages", "--prefix", "account/"]).await).len(), 3);
    let mailbox = cicada(&s, &["get", "mailbox", "account/acme:1"]).await;
    assert!(mailbox.starts_with("STATE") && mailbox.contains("pending") && mailbox.contains("in 59m"), "{mailbox}");

    assert_eq!(cicada(&s, &["cancel", "message", "m1"]).await, "message m1 cancelled\n");
    assert!(cicada_fails(&s, &["cancel", "message", "m1"]).await.contains("already finished"));

    assert_eq!(cicada(&s, &["pause", "account/acme:1"]).await, "actor account/acme:1 paused\n");
    assert_eq!(json(&s, &["get", "actor", "account/acme:1"]).await["paused"], true);
    assert_eq!(cicada(&s, &["deliver-now", "m2"]).await, "message m2 due now\n");
    assert_eq!(json(&s, &["get", "message", "m2"]).await["status"], "pending", "paused: nothing runs");
    assert_eq!(cicada(&s, &["resume", "account/acme:1"]).await, "actor account/acme:1 resumed\n");
    eventually_status(&s, "m2", "done").await;

    assert_eq!(cicada(&s, &["purge", "account/acme:1"]).await, "1 messages of actor account/acme:1 purged\n");
    let purged = cicada(&s, &["get", "outcomes", "--status", "purged"]).await;
    assert!(purged.contains("m3") && purged.contains("purged"), "{purged}");
    assert_eq!(cicada(&s, &["delete", "actor", "account/acme:1"]).await, "actor account/acme:1 deleted\n");
}

#[tokio::test]
async fn failed_messages_are_listed_and_retried() {
    let s = server().await;
    cicada(&s, &["create", "actor", "account/acme:2", "--state", "{}"]).await;
    for id in ["e1", "e2", "e3"] {
        cicada(&s, &["send", "account/acme:2", "--type", "Explode", "--id", id]).await;
        eventually_status(&s, id, "failed").await;
    }
    let failed = cicada(&s, &["get", "outcomes", "--status", "failed"]).await;
    assert_eq!(rows(&failed).len(), 3);
    assert!(failed.contains("boom"), "{failed}");
    for (since, count) in [("2000-01-01T00:00:00Z", 3), ("2999-01-01T00:00:00Z", 0)] {
        let outcomes = cicada(&s, &["get", "outcomes", "--status", "failed", "--since", since]).await;
        assert_eq!(rows(&outcomes).len(), count, "since {since}");
    }

    let retried = sent_id(&cicada(&s, &["retry", "message", "e1", "-o", "json"]).await);
    assert_ne!(retried, "e1");
    let caused = json(&s, &["get", "message", &retried]).await;
    assert_eq!(caused["message"]["causedBy"], "e1");

    let bulk = json(&s, &["retry", "messages", "--id", "e2", "--id", "e3"]).await;
    assert_eq!((bulk["affected"].as_u64(), bulk["more"].as_bool()), (Some(2), Some(false)));

    for id in ["d1", "d2"] {
        cicada(
            &s,
            &[
                "send",
                "account/acme:2",
                "--type",
                "Deposit",
                "--payload",
                r#"{"amount":1}"#,
                "--delay",
                "PT1H",
                "--id",
                id,
            ],
        )
        .await;
    }
    let cancelled = cicada(&s, &["cancel", "messages", "--prefix", "account/acme:2", "--type", "Deposit"]).await;
    assert_eq!(cancelled, "2 messages cancelled\n");
}

#[tokio::test]
async fn missing_items_and_api_errors_fail_with_a_message() {
    let s = server().await;
    assert_eq!(cicada_fails(&s, &["get", "actor", "account/nobody:1"]).await, "actor account/nobody:1 not found");
    assert_eq!(cicada_fails(&s, &["get", "message", "nope"]).await, "message nope not found");
    assert_eq!(cicada_fails(&s, &["get", "mailbox", "account/nobody:1"]).await, "actor account/nobody:1 not found");
    assert_eq!(cicada_fails(&s, &["get", "caused", "nope"]).await, "message nope not found");
    assert_eq!(cicada_fails(&s, &["retry", "message", "nope"]).await, "message nope not found");
    assert_eq!(cicada_fails(&s, &["pause", "account/nobody:1"]).await, "actor account/nobody:1 not found");
    assert!(cicada_fails(&s, &["cancel", "turn", "account/nobody:1"]).await.contains("no turn is running"));

    let unknown_type = cicada_fails(&s, &["create", "actor", "ghost/x", "--state", "{}"]).await;
    assert!(unknown_type.starts_with("404 "), "{unknown_type}");
    cicada(&s, &["create", "actor", "account/acme:3", "--state", "{}"]).await;
    let bad_message = cicada_fails(&s, &["send", "account/acme:3", "--type", "Nope"]).await;
    assert!(bad_message.starts_with("422 "), "{bad_message}");

    assert!(cicada_fails("http://127.0.0.1:1", &["health"]).await.contains("HTTP request failed"));
}

#[tokio::test]
async fn nodes_types_and_health() {
    let s = server().await;
    let nodes = json(&s, &["get", "nodes"]).await;
    let node = nodes[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(nodes[0]["status"], "alive");
    let table = cicada(&s, &["get", "node", &node, "-o", "wide"]).await;
    assert!(table.contains("HOSTED") && table.contains("account"), "{table}");
    assert!(cicada(&s, &["describe", "node", &node]).await.contains("turns: []"));
    assert!(rows(&cicada(&s, &["get", "turns"]).await).is_empty());
    assert!(cicada_fails(&s, &["get", "turn", "account/acme:1"]).await.contains("not found"));

    assert_eq!(rows(&cicada(&s, &["get", "types"]).await), ["account  tenant:id  3"]);
    assert!(rows(&cicada(&s, &["get", "type", "account", "-o", "wide"]).await)[0].starts_with("account  tenant:id  "));
    let account = json(&s, &["get", "type", "account"]).await;
    assert_eq!((account["name"].as_str(), account["messages"]["Hold"].is_object()), (Some("account"), true));
    assert_eq!(cicada_fails(&s, &["get", "type", "ghost"]).await, "type ghost not found");
    assert!(cicada(&s, &["describe", "type", "account"]).await.contains("Deposit:"));
    assert_eq!(cicada(&s, &["health"]).await, format!("{s} is healthy\n"));

    cicada(&s, &["create", "actor", "account/acme:4", "--state", "{}"]).await;
    assert!(cicada_fails(&s, &["delete", "type", "account"]).await.contains("in use"));
    cicada(&s, &["delete", "actor", "account/acme:4"]).await;
    assert_eq!(cicada(&s, &["delete", "type", "account"]).await, "type account removed\n");
    assert_eq!(cicada_fails(&s, &["delete", "type", "account"]).await, "type account not found");
}

#[tokio::test]
async fn a_running_turn_is_shown_and_cancelled() {
    let s = server().await;
    cicada(&s, &["create", "actor", "account/acme:5", "--state", "{}"]).await;
    cicada(&s, &["send", "account/acme:5", "--type", "Hold", "--id", "h1"]).await;
    eventually_status(&s, "h1", "running").await;

    let turn = json(&s, &["get", "turn", "account/acme:5"]).await;
    assert_eq!((turn["messageId"].as_str(), turn["cancelRequested"].as_bool()), (Some("h1"), Some(false)));
    assert_eq!(rows(&cicada(&s, &["get", "turns", "--prefix", "account/"]).await).len(), 1);
    let node = turn["node"].as_str().unwrap();
    let detail = json(&s, &["get", "node", node]).await;
    assert_eq!((detail["id"].as_str(), detail["turns"][0]["messageId"].as_str()), (Some(node), Some("h1")));
    let yaml = cicada(&s, &["get", "node", node, "-o", "yaml"]).await;
    assert!(yaml.contains("turns:") && yaml.contains("messageId: h1"), "{yaml}");

    let cancelled = cicada(&s, &["cancel", "turn", "account/acme:5"]).await;
    assert_eq!(cancelled, "turn on actor account/acme:5 asked to cancel\n");
    eventually_status(&s, "h1", "cancelled").await;
    assert!(cicada_fails(&s, &["get", "turn", "account/acme:5"]).await.contains("not found"));
}

#[tokio::test]
async fn a_listing_shows_one_page_unless_all() {
    let s = server().await;
    for id in 1..=3 {
        cicada(&s, &["create", "actor", &format!("account/page:{id}"), "--state", "{}"]).await;
    }
    assert_eq!(rows(&cicada(&s, &["get", "actors", "--limit", "2"]).await).len(), 2);
    assert_eq!(rows(&cicada(&s, &["get", "actors", "--limit", "2", "--all"]).await).len(), 3);
    assert_eq!(json(&s, &["get", "actors", "--limit", "1", "--all"]).await.as_array().unwrap().len(), 3);
}

/// A node hosting `maintenance.cleanup` that counts the requests it gets; its base URL and the count.
async fn counting_cleanup_server() -> (String, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let count = middleware::from_fn(move |request: Request, next: Next| {
        counter.fetch_add(1, Ordering::SeqCst);
        next.run(request)
    });
    (server_hosting(|node| node.host(CleanupActor), |app| app.layer(count)).await, requests)
}

#[tokio::test]
async fn the_command_line_without_a_state_names_it_and_sends_no_request() {
    let (s, requests) = counting_cleanup_server().await;
    let error = cicada_fails(&s, &["create", "actor", "maintenance.cleanup/_"]).await;
    assert!(error.contains("--state"), "{error}");
    assert_eq!(requests.load(Ordering::SeqCst), 0, "no request reached the node");
}

#[tokio::test]
async fn the_command_line_with_a_state_creates_the_actor_with_it() {
    let (s, _) = counting_cleanup_server().await;
    let created = cicada(&s, &["create", "actor", "maintenance.cleanup/_", "--state", "{}"]).await;
    assert_eq!(created, "actor maintenance.cleanup/_ created\n");
    assert_eq!(json(&s, &["get", "actor", "maintenance.cleanup/_"]).await["state"], serde_json::json!({}));
}

async fn require_token(request: Request, next: Next) -> Response {
    match request.headers().get(header::AUTHORIZATION) {
        Some(value) if value == "Bearer t1" => next.run(request).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

#[tokio::test]
async fn contexts_come_from_the_config_file_and_carry_the_token() {
    let s = server_with(|app| app.layer(middleware::from_fn(require_token))).await;
    let dir = tempfile::tempdir().unwrap();
    let env = Env::from([("CICADA_CONFIG".to_owned(), dir.path().join("config.toml").display().to_string())]);

    let error = cicada_in(&env, &["health"]).await.unwrap_err();
    assert!(error.contains("no server"), "{error}");
    assert!(cicada_in(&env, &["config", "set-context", "x"]).await.unwrap_err().contains("--server"));

    cicada_in(&env, &["config", "set-context", "local", "--server", &s, "--token", "t1"]).await.unwrap();
    cicada_in(&env, &["config", "set-context", "other", "--server", "http://127.0.0.1:1"]).await.unwrap();
    assert_eq!(cicada_in(&env, &["config", "current-context"]).await.unwrap(), "local\n", "the first becomes current");
    let contexts = cicada_in(&env, &["config", "get-contexts"]).await.unwrap();
    assert!(contexts.contains(&format!("*        local  {s}")), "{contexts}");

    assert_eq!(cicada_in(&env, &["health"]).await.unwrap(), format!("{s} is healthy\n"));
    let refused = cicada_in(&env, &["--token", "wrong", "health"]).await.unwrap_err();
    assert!(refused.starts_with("401 "), "{refused}");

    assert!(cicada_in(&env, &["config", "use-context", "nope"]).await.unwrap_err().contains("'nope'"));
    cicada_in(&env, &["config", "use-context", "other"]).await.unwrap();
    assert!(cicada_in(&env, &["health"]).await.unwrap_err().contains("HTTP request failed"));
    assert!(cicada_in(&env, &["--context", "local", "health"]).await.is_ok());
    assert!(cicada_in(&env, &["--context", "nope", "health"]).await.unwrap_err().contains("'nope'"));
}

/// `cicada tui` without a terminal is an error, not a panic. Skipped where `/dev/tty` opens, so it never takes over a
/// developer's screen; it runs in CI, which has no terminal.
#[cfg(unix)]
#[test]
fn the_tui_without_a_terminal_fails_cleanly() {
    if std::fs::File::open("/dev/tty").is_ok() {
        eprintln!("skipped: a terminal is attached");
        return;
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_cicada"))
        .args(["--server", "http://127.0.0.1:1", "tui"])
        .env("CICADA_CONFIG", "/nonexistent/cicada/config.toml")
        .env_remove("CICADA_CONTEXT")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("cicada runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.starts_with("error: cannot start the terminal UI"), "{stderr}");
    assert!(output.stdout.is_empty(), "stdout: {:?}", String::from_utf8_lossy(&output.stdout));
}
