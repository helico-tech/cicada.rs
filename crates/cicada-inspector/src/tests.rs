//! The model against an in-process server: `cicada-axum` on a `TestActors` store, with some routes replaced.
//!
//! These tests use real sockets and the real clock: Tokio's paused clock would advance through the client's timeouts
//! while a socket waits. Deadlines are short to keep them fast.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};
use cicada::prelude::*;
use cicada_app::Env;
use cicada_axum::actor_api;
use cicada_client::ActorClient;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::model::{
    Count, Limits, Navigation, Part, Section, Session, TIMED_OUT, TypeRow, count_with, overview, tiles,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Counter {
    id: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = Work;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Syncer {
    id: String,
}

impl ActorRef for Syncer {
    const TYPE: &'static str = "sync";
    type Message = Work;
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(tag = "type")]
enum Work {
    Ok,
    Fail,
}

struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: Work) -> HandlerResult {
        Ok(())
    }
}

struct SyncActor;

impl Actor for SyncActor {
    type Ref = Syncer;
    type State = NoState;

    async fn handle(&self, _: &mut Turn<Self>, message: Work) -> HandlerResult {
        match message {
            Work::Ok => Ok(()),
            Work::Fail => Err(HandlerError::permanent("broken")),
        }
    }
}

/// What a replaced route does.
#[derive(Clone)]
enum Fake {
    /// Answers this status and JSON body.
    Answer(StatusCode, Value),
    /// Never answers.
    Hang,
    /// Signals `arrived` when the request comes in, then answers after `release` fires.
    Wait { arrived: Arc<Notify>, release: Arc<Notify> },
}

type Rule = Arc<dyn Fn(&str) -> Option<Fake> + Send + Sync>;

/// A server for `actors` whose requests go through `rule` first, by path and query; it counts the per-type `/stats`
/// calls in flight in `in_flight` and keeps their most in `most`, holding each for `hold`.
struct Server {
    client: ActorClient,
    url: String,
    most: Arc<AtomicUsize>,
}

async fn serve(actors: &TestActors, rule: Rule, hold: Duration) -> Server {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let (counting, top) = (in_flight.clone(), most.clone());
    let layer = from_fn(move |request: Request, next: Next| {
        let (rule, counting, top) = (rule.clone(), counting.clone(), top.clone());
        async move {
            let target = request.uri().path_and_query().map_or_else(String::new, |p| p.as_str().to_owned());
            let per_type = target.starts_with("/stats?prefix=") && target.len() > "/stats?prefix=".len();
            if per_type {
                let now = counting.fetch_add(1, Ordering::SeqCst) + 1;
                top.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(hold).await;
            }
            let response = match rule(&target) {
                Some(Fake::Answer(status, body)) => {
                    let mut response = Response::new(Body::from(body.to_string()));
                    *response.status_mut() = status;
                    let json = if status.is_success() { "application/json" } else { "application/problem+json" };
                    response.headers_mut().insert(header::CONTENT_TYPE, json.parse().unwrap());
                    response
                }
                Some(Fake::Hang) => std::future::pending::<Response>().await,
                Some(Fake::Wait { arrived, release }) => {
                    arrived.notify_one();
                    release.notified().await;
                    next.run(request).await
                }
                None => next.run(request).await.into_response(),
            };
            if per_type {
                counting.fetch_sub(1, Ordering::SeqCst);
            }
            response
        }
    });
    let app: Router = actor_api(actors.system().clone()).layer(layer);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    Server { client: ActorClient::new(&url), url, most }
}

fn no_rule() -> Rule {
    Arc::new(|_| None)
}

async fn actors() -> TestActors {
    TestActors::builder().with(|node| node.host(CounterActor).host(SyncActor)).build().await.unwrap()
}

fn problem(status: u16) -> Fake {
    let body = json!({"type": "about:blank", "title": "Internal Server Error", "status": status});
    Fake::Answer(StatusCode::from_u16(status).unwrap(), body)
}

/// Scenario: An Overview.
#[tokio::test]
async fn an_overview_shows_the_counts_health_nodes_turns_and_types() {
    let actors = actors().await;
    for id in ["a", "b"] {
        actors.create(&Counter { id: id.into() }, &json!({})).await;
    }
    let sync = Syncer { id: "s".into() };
    actors.create(&sync, &json!({})).await;
    actors.send(&sync, Work::Fail).await;
    actors.run_until_idle().await;
    actors.send_with(&Counter { id: "a".into() }, Work::Ok, SendOptions::new().delay(Duration::from_secs(60))).await;
    let server = serve(&actors, no_rule(), Duration::ZERO).await;

    let overview = overview(&server.client, Limits::DEFAULT).await;
    let stats = overview.stats.ready().expect("stats");
    assert_eq!((stats.actors, stats.pending, stats.failed), (3, 1, Some(1)));
    assert_eq!(overview.health, Part::Ready(true));
    assert!(overview.nodes.ready().is_some(), "{:?}", overview.nodes);
    let turns = overview.turns.ready().expect("turns");
    assert!(turns.items.is_empty() && !turns.more);
    let types = overview.types.ready().expect("types");
    assert_eq!(
        types,
        &[
            TypeRow { name: "counter".into(), actors: Count::Counted(2) },
            TypeRow { name: "sync".into(), actors: Count::Counted(1) },
        ]
    );
    let [actors_tile, pending, _, failed] = tiles(&overview.stats, actors.now());
    assert_eq!((actors_tile.value.as_str(), pending.value.as_str()), ("3", "1"));
    assert_eq!((failed.value.as_str(), failed.note.as_str()), ("1", "kept by retention"));
}

/// The first page of `/turns` is 20 turns, with a mark when the server has more.
#[tokio::test]
async fn the_running_turns_are_the_first_page_with_a_more_mark() {
    let actors = actors().await;
    let turn = |n: usize| {
        json!({"actor": format!("counter/{n}"), "messageId": format!("m{n}"), "node": "n1",
               "leaseUntil": "2026-11-27T10:00:00.000Z", "cancelRequested": false})
    };
    let rule: Rule = Arc::new(move |target| {
        let page = json!({"items": (0..20).map(turn).collect::<Vec<_>>(), "next": "cursor"});
        (target == "/turns?limit=20").then(|| Fake::Answer(StatusCode::OK, page))
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let overview = overview(&server.client, Limits::DEFAULT).await;
    let turns = overview.turns.ready().unwrap_or_else(|| panic!("{:?}", overview.turns));
    assert_eq!((turns.items.len(), turns.more), (20, true));
}

/// Scenario: One route fails.
#[tokio::test]
async fn a_failing_route_marks_only_its_part() {
    let actors = actors().await;
    let rule: Rule = Arc::new(|target| (target == "/nodes").then(|| problem(500)));
    let server = serve(&actors, rule, Duration::ZERO).await;
    let overview = overview(&server.client, Limits::DEFAULT).await;
    let Part::Failed(error) = &overview.nodes else { panic!("{:?}", overview.nodes) };
    assert!(error.contains("500"), "{error}");
    assert!(overview.stats.ready().is_some() && overview.health.ready().is_some());
    assert!(overview.turns.ready().is_some() && overview.types.ready().is_some());
}

/// Scenario: An older server.
#[tokio::test]
async fn failed_from_an_older_server_is_none_and_shows_a_dash() {
    let actors = actors().await;
    let older = json!({"actors": 4, "pending": 0, "oldestDue": null});
    let rule: Rule = Arc::new(move |target| (target == "/stats").then(|| Fake::Answer(StatusCode::OK, older.clone())));
    let server = serve(&actors, rule, Duration::ZERO).await;
    let overview = overview(&server.client, Limits::DEFAULT).await;
    assert_eq!(overview.stats.ready().map(|s| (s.actors, s.failed)), Some((4, None)));
    assert_eq!(tiles(&overview.stats, actors.now())[3].value, "—");
}

fn types_answer(names: impl IntoIterator<Item = String>) -> Fake {
    let items: Vec<Value> =
        names.into_iter().map(|name| json!({"name": name, "hash": "sha256:0", "state": {}, "messages": {}})).collect();
    Fake::Answer(StatusCode::OK, json!({ "items": items }))
}

/// Scenario: Many types; and at most 8 counts at a time.
#[tokio::test]
async fn sixty_types_count_fifty_at_most_eight_at_a_time() {
    let actors = actors().await;
    let answer = types_answer((0..60).map(|n| format!("t{n:02}")));
    let rule: Rule = Arc::new(move |target| (target == "/types").then(|| answer.clone()));
    let server = serve(&actors, rule, Duration::from_millis(20)).await;
    let overview = overview(&server.client, Limits::DEFAULT).await;
    let types = overview.types.ready().expect("types");
    assert_eq!(types.len(), 60);
    assert!(types[..50].iter().all(|row| row.actors == Count::Counted(0)), "{types:?}");
    assert!(types[50..].iter().all(|row| row.actors == Count::NotCounted), "{types:?}");
    let most = server.most.load(Ordering::SeqCst);
    assert!((2..=8).contains(&most), "{most} counts at a time");
}

/// At the deadline the answers that came are kept, a base route still out has failed and a type still uncounted is
/// not counted; a count that failed is failed, not "not counted". The stalled routes never answer and the deadline is
/// far beyond a local round trip, so the result does not depend on the machine's speed.
#[tokio::test]
async fn the_deadline_keeps_what_answered() {
    let actors = actors().await;
    let answer = types_answer(["fast", "broken", "slow"].map(String::from));
    let rule: Rule = Arc::new(move |target| match target {
        "/types" => Some(answer.clone()),
        "/stats?prefix=broken%2F" => Some(problem(500)),
        "/stats?prefix=slow%2F" | "/nodes" => Some(Fake::Hang),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let limits = Limits { deadline: Duration::from_secs(2), ..Limits::DEFAULT };
    let overview = timeout(Duration::from_secs(20), overview(&server.client, limits)).await.expect("bounded");
    assert_eq!(overview.nodes, Part::Failed(TIMED_OUT.into()));
    assert!(overview.stats.ready().is_some() && overview.health.ready().is_some());
    let types = overview.types.ready().expect("types");
    assert_eq!(types[0].actors, Count::Counted(0));
    assert!(matches!(&types[1].actors, Count::Failed(error) if error.contains("500")), "{types:?}");
    assert_eq!(types[2].actors, Count::NotCounted);
}

/// A count still queued at the deadline never starts: two counts answer, the next eight stall and fill every slot,
/// and the forty behind them are not counted without a call. On Tokio's paused clock, without sockets.
#[tokio::test(start_paused = true)]
async fn a_count_queued_at_the_deadline_never_starts() {
    let started = std::sync::Mutex::new(Vec::new());
    let names: Vec<String> = (0..50).map(|n| format!("t{n:02}")).collect();
    let deadline = tokio::time::Instant::now() + Limits::DEFAULT.deadline;
    let rows = count_with(names, deadline, Limits::DEFAULT, |name: String| {
        let answers = name.as_str() < "t02";
        started.lock().unwrap().push(name);
        async move { if answers { Ok(1) } else { std::future::pending().await } }
    })
    .await;
    assert!(tokio::time::Instant::now() >= deadline);
    let started = started.into_inner().unwrap();
    assert_eq!(started, (0..10).map(|n| format!("t{n:02}")).collect::<Vec<_>>(), "no count starts after the deadline");
    assert!(rows[..2].iter().all(|row| row.actors == Count::Counted(1)), "{rows:?}");
    assert!(rows[2..].iter().all(|row| row.actors == Count::NotCounted), "{rows:?}");
}

fn config(dir: &tempfile::TempDir, contexts: &[(&str, &str)]) -> Env {
    let file = dir.path().join("config.toml");
    let mut text = format!("current-context = \"{}\"\n", contexts[0].0);
    for (name, server) in contexts {
        text.push_str(&format!("[contexts.{name}]\nserver = \"{server}\"\n"));
    }
    std::fs::write(&file, text).unwrap();
    Env::from([("CICADA_CONFIG".to_owned(), file.to_str().unwrap().to_owned())])
}

/// A refresh of one context that is still in flight when the user switches to another is dropped: its `/stats` is
/// stalled at the server when the switch happens, and after it is released only the new context's Overview shows.
#[tokio::test]
async fn a_refresh_in_flight_at_a_context_switch_is_dropped() {
    let (slow_actors, fast_actors) = (actors().await, actors().await);
    slow_actors.create(&Counter { id: "old".into() }, &json!({})).await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let stall = Fake::Wait { arrived: arrived.clone(), release: release.clone() };
    let slow = serve(&slow_actors, Arc::new(move |t| (t == "/stats").then(|| stall.clone())), Duration::ZERO).await;
    let fast = serve(&fast_actors, no_rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let env = config(&dir, &[("slow", &slow.url), ("fast", &fast.url)]);
    let session = Session::new(env, tokio::runtime::Handle::current(), Limits::DEFAULT);
    let mut view = session.view();

    session.use_context(None);
    assert_eq!(view.borrow().context.as_deref(), Some("slow"));
    timeout(Duration::from_secs(10), arrived.notified()).await.expect("the old refresh is in flight");
    session.use_context(Some("fast".into()));
    let done =
        timeout(Duration::from_secs(10), view.wait_for(|v| v.overview.is_some())).await.expect("refreshed").unwrap();
    assert_eq!(done.context.as_deref(), Some("fast"));
    assert_eq!(done.overview.as_ref().unwrap().stats.ready().map(|s| s.actors), Some(0));
    drop(done);

    // The old store has one actor: had its refresh published, the view would change and show it.
    release.notify_waiters();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!view.has_changed().unwrap(), "nothing is published after the new Overview");
    let view = view.borrow();
    assert_eq!(view.overview.as_ref().unwrap().stats.ready().map(|s| s.actors), Some(0), "the old refresh is dropped");
    assert_eq!((view.context.as_deref(), view.picked.as_deref()), (Some("fast"), Some("fast")));
}

/// The review's malformed server URL with a password: no part of the Overview shows the password.
#[tokio::test]
async fn a_password_in_a_malformed_server_url_never_reaches_the_window() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let dir = tempfile::tempdir().unwrap();
    let mut env = config(&dir, &[("prod", "http://unused")]);
    env.insert("CICADA_SERVER".into(), format!("http://u:dummy secret@{address} "));
    let session = Session::new(env, tokio::runtime::Handle::current(), Limits::DEFAULT);
    session.use_context(None);
    let mut view = session.view();
    let done =
        timeout(Duration::from_secs(10), view.wait_for(|v| v.overview.is_some())).await.expect("refreshed").unwrap();
    let text = format!("{:?}", *done);
    assert!(matches!(done.overview.as_ref().unwrap().stats, Part::Failed(_)), "{text}");
    assert!(!text.contains("dummy") && !text.contains("secret"), "{text}");
}

/// A context without a server leaves the window usable: the error, no Overview, the contexts still listed; the
/// environment's server shows as an override.
#[tokio::test]
async fn a_target_error_is_shown_and_overrides_are_named() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = config(&dir, &[("prod", "not a url")]);
    let session = Session::new(env.clone(), tokio::runtime::Handle::current(), Limits::DEFAULT);
    session.use_context(None);
    let view = session.view().borrow().clone();
    assert!(view.target_error.as_deref().is_some_and(|e| e.contains("server URL")), "{:?}", view.target_error);
    assert_eq!(view.contexts.names(), ["prod"]);
    assert!(view.overview.is_none() && !view.refreshing && view.overrides.is_empty());

    let actors = actors().await;
    let server = serve(&actors, no_rule(), Duration::ZERO).await;
    env.insert("CICADA_SERVER".into(), server.url.clone());
    let session = Session::new(env, tokio::runtime::Handle::current(), Limits::DEFAULT);
    session.use_context(Some("prod".into()));
    let mut view = session.view();
    let done =
        timeout(Duration::from_secs(5), view.wait_for(|v| v.overview.is_some())).await.expect("refreshed").unwrap();
    assert_eq!((done.overrides.as_slice(), done.context.as_deref()), (&["CICADA_SERVER"][..], Some("prod")));
    assert!(done.target_error.is_none());
}

/// Scenario: Back and forward.
#[test]
fn back_and_forward_follow_the_selection() {
    let mut navigation = Navigation::default();
    assert!(!navigation.can_back() && !navigation.can_forward());
    navigation.select(Section::Overview);
    navigation.select(Section::Nodes);
    assert!(navigation.back());
    assert_eq!(navigation.current(), Section::Overview);
    assert!(navigation.can_forward());
    navigation.select(Section::Types);
    assert!(!navigation.can_forward(), "a new selection drops the forward list");
    assert!(navigation.back());
    assert_eq!(navigation.current(), Section::Overview);
    assert!(navigation.forward());
    assert_eq!(navigation.current(), Section::Types);
    assert!(!navigation.forward());
}

/// A failed `/stats` puts its error on every tile; an error text names the error and its causes.
#[test]
fn a_failed_stats_shows_its_error_on_the_tiles() {
    let cause = std::io::Error::other("connection refused");
    let error = crate::model::error_text(&std::io::Error::other(OuterError(cause)));
    assert_eq!(error, "cannot connect: connection refused");
    let tiles = tiles(&Part::Failed(error.clone()), chrono::Utc::now());
    assert!(tiles.iter().all(|tile| tile.error && tile.value == "—" && tile.note == error));
    assert!(Section::BROWSE.iter().all(|section| !section.ready()) && Section::Overview.ready());
    assert_eq!(crate::model::check_macos((13, 6, 0)), Err(crate::model::NEEDS_MACOS));
    assert_eq!(crate::model::check_macos((14, 0, 0)), Ok(()));
}

/// An error with a cause, for the error text.
#[derive(Debug)]
struct OuterError(std::io::Error);

impl std::fmt::Display for OuterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cannot connect")
    }
}

impl std::error::Error for OuterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}
