//! The Nodes page's model against an in-process server: the cache and when it is read, the local filter, a failed
//! read, and the pane of the selected node with its turns. Real sockets and the real clock, as in the parent module,
//! but no outcome depends on the machine's speed: a read's deadline is far past every wait (but in the deadline test,
//! where the read never answers), answers that must come in an order are held, and every wait has a bound.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::runtime::NodeHealth;
use cicada::store::NodeId;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use tokio::time::timeout;

use super::{Counter, Fake, Rule, Server, actors, config, problem, serve};
use crate::model::{Limits, Navigation, Section, Session, TIMED_OUT};
use crate::nodes::{
    NodePane, NodeRead, Nodes, NodesRead, NodesView, column_text, health_text, hosted_text, precise_time,
};

/// The longest a test waits for one change.
const WAIT: Duration = Duration::from_secs(10);

/// A read times out only well after a wait would have failed.
fn limits() -> Limits {
    Limits { debounce: Duration::ZERO, deadline: 6 * WAIT, ..Limits::DEFAULT }
}

fn model(server: &Server, limits: Limits) -> Nodes {
    let nodes = Nodes::new(tokio::runtime::Handle::current(), limits);
    nodes.use_client(Some(server.client.clone()));
    nodes
}

async fn until<T: Clone>(view: &mut watch::Receiver<T>, done: impl FnMut(&T) -> bool) -> T {
    timeout(WAIT, view.wait_for(done)).await.expect("the model changes in time").unwrap().clone()
}

/// A served node.
fn node(id: &str, status: &str) -> Value {
    json!({"id": id, "hosted": ["counter"], "startedAt": "2026-11-27T09:00:00.000Z",
           "lastSeen": "2026-11-27T09:59:30.123Z", "status": status, "runningTurns": 0, "hostname": "box-1",
           "label": "blue"})
}

fn nodes_answer(items: &[Value]) -> Fake {
    Fake::Answer(StatusCode::OK, json!({ "items": items }))
}

/// A served `GET /nodes/{node}` with the turns of `actors`.
fn detail(id: &str, label: &str, actors: &[&str]) -> Fake {
    let mut body = node(id, "alive");
    body["label"] = json!(label);
    body["runningTurns"] = json!(actors.len());
    body["turns"] = actors
        .iter()
        .enumerate()
        .map(|(n, actor)| {
            json!({"actor": actor, "messageId": format!("m{n}"), "node": id, "leaseUntil": "2026-11-27T10:00:00.000Z",
                   "cancelRequested": n == 1})
        })
        .collect();
    Fake::Answer(StatusCode::OK, body)
}

fn no_node() -> Fake {
    let body = json!({"type": "urn:cicada:problem:no-such-node", "title": "Not Found", "status": 404});
    Fake::Answer(StatusCode::NOT_FOUND, body)
}

fn ids(view: &NodesView) -> Vec<String> {
    view.rows.iter().map(|row| row.id.to_string()).collect()
}

fn node_id(id: &str) -> NodeId {
    NodeId::try_from(id.to_owned()).unwrap()
}

fn label(pane: &NodePane) -> Option<String> {
    match &pane.read {
        NodeRead::Found(node) => node.node.label.clone(),
        _ => None,
    }
}

fn ready(view: &NodesView) -> bool {
    matches!(view.read, NodesRead::Ready(_))
}

fn served(server: &mut Server, target: &str) -> usize {
    std::iter::from_fn(|| server.events.try_recv().ok()).filter(|e| e == &format!("served {target}")).count()
}

/// The cache is read on the first visit and on the page's Refresh, never on the Overview's refresh; after a context
/// switch it is read at once when the page is visible, else at its next visit. Status and the last heartbeat show as
/// the server gave them.
#[tokio::test]
async fn the_cache_is_read_on_the_first_visit_its_refresh_and_a_context_switch_never_on_the_overview() {
    let (prod_actors, staging_actors) = (actors().await, actors().await);
    let prod_nodes = nodes_answer(&[node("n1", "alive"), node("n2", "dead")]);
    let staging_nodes = nodes_answer(&[node("s1", "suspect")]);
    let prod: Rule = Arc::new(move |target| (target == "/nodes").then(|| prod_nodes.clone()));
    let staging: Rule = Arc::new(move |target| (target == "/nodes").then(|| staging_nodes.clone()));
    let prod = serve(&prod_actors, prod, Duration::ZERO).await;
    let staging = serve(&staging_actors, staging, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let env = config(&dir, &[("prod", &prod.url), ("staging", &staging.url)]);
    let session = Session::new(env, tokio::runtime::Handle::current(), limits());
    let (nodes, mut list, mut view) = (session.nodes(), session.nodes().list(), session.view());
    let mut navigation = Navigation::default();
    session.use_context(None);
    until(&mut view, |v| v.overview.is_some()).await;
    assert_eq!(list.borrow().read, NodesRead::NotRead, "the Overview's refresh leaves the page alone");

    nodes.show();
    let first = until(&mut list, ready).await;
    assert_eq!(ids(&first), ["n1", "n2"]);
    let dead = &first.rows[1];
    assert_eq!(
        (dead.status, dead.label.as_deref(), dead.hostname.as_deref()),
        (NodeHealth::Dead, Some("blue"), Some("box-1"))
    );
    assert_eq!(precise_time(dead.last_seen), "2026-11-27 09:59:30.123 UTC", "the heartbeat keeps its fraction");
    assert_eq!(precise_time(dead.started_at), "2026-11-27 09:00:00 UTC");
    let cells: Vec<_> = ["id", "status", "seen"].map(|column| column_text(dead, column)).into();
    assert_eq!(cells, ["n2", "dead", "2026-11-27 09:59:30.123 UTC"], "the table's cells");
    assert_eq!(
        (health_text(dead.status), hosted_text(&dead.hosted), hosted_text(&[])),
        ("dead", "counter".into(), "sends only".into())
    );
    nodes.hide();
    nodes.show();
    session.refresh();
    until(&mut view, |v| v.overview.is_some() && !v.refreshing).await;
    assert_eq!(list.borrow().generation, first.generation, "neither a visit nor the Overview reads it again");

    nodes.refresh();
    assert_eq!(list.borrow().read, NodesRead::Loading);
    let refreshed = until(&mut list, ready).await;
    assert!(refreshed.generation > first.generation);

    session.switch_context(&mut navigation, Some("staging".into()));
    let switched = until(&mut list, ready).await;
    assert_eq!(ids(&switched), ["s1"], "a visible page reads at once");
    nodes.hide();
    session.switch_context(&mut navigation, Some("prod".into()));
    until(&mut view, |v| v.overview.is_some() && !v.refreshing).await;
    assert_eq!((list.borrow().read.clone(), list.borrow().rows.len()), (NodesRead::NotRead, 0), "a hidden page waits");
    nodes.show();
    assert_eq!(ids(&until(&mut list, ready).await), ["n1", "n2"]);
    session.stop();
}

/// A list read still out at a context switch is dropped: its answer, released after the new context's list landed,
/// publishes nothing.
#[tokio::test]
async fn a_late_list_result_after_a_context_switch_is_dropped() {
    let (prod_actors, staging_actors) = (actors().await, actors().await);
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held = Fake::Later {
        arrived: arrived.clone(),
        release: release.clone(),
        status: StatusCode::OK,
        body: json!({"items": [node("old", "alive")]}),
    };
    let staging_nodes = nodes_answer(&[node("s1", "alive")]);
    let prod = serve(&prod_actors, Arc::new(move |t| (t == "/nodes").then(|| held.clone())), Duration::ZERO).await;
    let staging =
        serve(&staging_actors, Arc::new(move |t| (t == "/nodes").then(|| staging_nodes.clone())), Duration::ZERO).await;
    let nodes = model(&prod, limits());
    let mut list = nodes.list();
    nodes.show();
    timeout(WAIT, arrived.notified()).await.expect("the prod read is held");
    nodes.take_aborted();
    nodes.use_client(Some(staging.client.clone()));
    let view = until(&mut list, ready).await;
    assert_eq!(ids(&view), ["s1"]);
    release.notify_one();
    let aborted = nodes.take_aborted();
    assert!(!aborted.is_empty());
    for task in aborted {
        let ended = timeout(WAIT, task).await.expect("the dropped read ends");
        assert!(ended.as_ref().is_err_and(|error| error.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    assert!(!list.has_changed().unwrap(), "the late answer publishes nothing");
    assert_eq!(ids(&list.borrow()), ["s1"]);
}

/// A failed read shows its error and keeps no stale list; Retry reads again. A read with no answer fails at the
/// shared read deadline.
#[tokio::test]
async fn a_failed_read_keeps_no_stale_list_and_is_retried() {
    let actors = actors().await;
    let mode = Arc::new(AtomicUsize::new(0));
    let rule_mode = mode.clone();
    let rule: Rule = Arc::new(move |target| match (target, rule_mode.load(Ordering::SeqCst)) {
        ("/nodes", 0) => Some(nodes_answer(&[node("n1", "alive")])),
        ("/nodes", 1) => Some(problem(500)),
        (target, _) if target.starts_with("/nodes") => Some(Fake::Hang),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let nodes = model(&server, limits());
    let mut list = nodes.list();
    nodes.show();
    until(&mut list, ready).await;
    mode.store(1, Ordering::SeqCst);
    nodes.refresh();
    let failed = until(&mut list, |l| matches!(l.read, NodesRead::Failed(_))).await;
    assert!(matches!(&failed.read, NodesRead::Failed(error) if error.contains("500")), "{:?}", failed.read);
    assert_eq!((failed.rows.len(), failed.loaded), (0, 0), "no stale list");
    mode.store(0, Ordering::SeqCst);
    nodes.retry();
    assert_eq!(ids(&until(&mut list, ready).await), ["n1"]);

    mode.store(2, Ordering::SeqCst);
    let short = model(&server, Limits { deadline: Duration::from_millis(200), ..limits() });
    let mut list = short.list();
    short.show();
    let timed_out = until(&mut list, |l| matches!(l.read, NodesRead::Failed(_))).await;
    assert_eq!(timed_out.read, NodesRead::Failed(TIMED_OUT.into()));
    short.select(Some(&node_id("n1")));
    let mut pane = short.pane();
    let pane = until(&mut pane, |p| matches!(p.read, NodeRead::Failed(_))).await;
    assert_eq!(pane.read, NodeRead::Failed(TIMED_OUT.into()), "the pane's read has the same deadline");
}

/// The search field filters the cache by id prefix with no read; a selected node it hides leaves the pane, and its
/// read still out is dropped.
#[tokio::test]
async fn the_filter_is_local_and_hiding_the_selection_drops_its_read() {
    let actors = actors().await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held =
        Fake::Later { arrived: arrived.clone(), release: release.clone(), status: StatusCode::OK, body: json!({}) };
    let all = nodes_answer(&[node("web-1", "alive"), node("web-2", "alive"), node("job-1", "suspect")]);
    let rule: Rule = Arc::new(move |target| match target {
        "/nodes" => Some(all.clone()),
        "/nodes/web-2" => Some(held.clone()),
        _ => None,
    });
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let nodes = model(&server, limits());
    let (mut list, mut pane) = (nodes.list(), nodes.pane());
    nodes.show();
    until(&mut list, ready).await;
    nodes.search("web-");
    let view = list.borrow_and_update().clone();
    assert_eq!((ids(&view), view.loaded, view.prefix.as_str()), (vec!["web-1".to_owned(), "web-2".into()], 3, "web-"));
    nodes.select(Some(&node_id("web-2")));
    timeout(WAIT, arrived.notified()).await.expect("the node read is held");
    nodes.take_aborted();
    nodes.search("job");
    assert_eq!(ids(&list.borrow()), ["job-1"]);
    assert_eq!((nodes.selected(), pane.borrow_and_update().node.clone()), (None, None));
    release.notify_one();
    for task in nodes.take_aborted() {
        let ended = timeout(WAIT, task).await.expect("the dropped read ends");
        assert!(ended.as_ref().is_err_and(|error| error.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    assert!(!pane.has_changed().unwrap(), "the dropped read publishes nothing");
    assert_eq!(served(&mut server, "/nodes"), 1, "the filter reads nothing");
}

/// Scenario: A node's turns. The pane lists the node's two turns, a cancel request marked; Show Actor on one selects
/// its actor on the Actors page. Back to the Nodes place selects the node and reads its pane fresh.
#[tokio::test]
async fn a_nodes_pane_lists_its_turns_and_back_reads_it_fresh() {
    let actors = actors().await;
    for id in ["a", "b"] {
        actors.create(&Counter { id: id.into() }, &json!({})).await;
    }
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = reads.clone();
    let rule: Rule = Arc::new(move |target| match target {
        "/nodes" => Some(nodes_answer(&[node("n1", "alive")])),
        "/nodes/n1" => {
            let n = counted.fetch_add(1, Ordering::SeqCst);
            Some(detail("n1", &format!("read {n}"), &["counter/a", "counter/b"]))
        }
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(config(&dir, &[("prod", &server.url)]), tokio::runtime::Handle::current(), limits());
    session.use_context(None);
    let mut navigation = Navigation::default();
    let nodes = session.nodes();
    let (mut list, mut pane) = (nodes.list(), nodes.pane());
    session.navigate(&mut navigation, |navigation| {
        navigation.select(Section::Nodes);
        true
    });
    nodes.show();
    until(&mut list, ready).await;
    nodes.select(Some(&node_id("n1")));
    let shown = until(&mut pane, |p| matches!(p.read, NodeRead::Found(_))).await;
    let NodeRead::Found(detail) = &shown.read else { unreachable!() };
    let turns: Vec<_> = detail.turns.iter().map(|t| (t.actor.to_string(), t.cancel_requested)).collect();
    assert_eq!(turns, [("counter/a".to_owned(), false), ("counter/b".to_owned(), true)]);
    assert_eq!(label(&shown).as_deref(), Some("read 0"));

    let b = detail.turns[1].actor.clone();
    let mut actor_pane = session.actors().pane();
    session.show_actor(&mut navigation, b.clone());
    until(&mut actor_pane, |p| p.address.as_ref() == Some(&b) && !p.not_found()).await;
    assert_eq!(session.actors().selected().map(|path| path.address().clone()), Some(b));

    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!((navigation.current(), nodes.selected()), (Section::Nodes, Some(node_id("n1"))));
    let fresh = until(&mut pane, |p| label(p).as_deref() == Some("read 1")).await;
    assert_eq!(fresh.node, Some(node_id("n1")));
    session.stop();
}

/// Scenario: A gone node. A node whose row was removed says "not found". A failed read shows its error, and Retry
/// reads the selected node again.
#[tokio::test]
async fn a_gone_node_says_not_found_and_a_failed_read_is_retried() {
    let actors = actors().await;
    let fails = Arc::new(AtomicUsize::new(1));
    let failing = fails.clone();
    let rule: Rule = Arc::new(move |target| match target {
        "/nodes/gone" => Some(no_node()),
        "/nodes/n1" if failing.fetch_sub(1, Ordering::SeqCst) == 1 => Some(problem(500)),
        "/nodes/n1" => Some(detail("n1", "back", &[])),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let nodes = model(&server, limits());
    let mut pane = nodes.pane();
    nodes.select(Some(&node_id("gone")));
    assert_eq!(until(&mut pane, |p| p.read != NodeRead::Loading).await.read, NodeRead::NotFound);
    nodes.select(Some(&node_id("n1")));
    let failed = until(&mut pane, |p| matches!(p.read, NodeRead::Failed(_))).await;
    assert!(matches!(&failed.read, NodeRead::Failed(error) if error.contains("500")), "{:?}", failed.read);
    nodes.retry_node();
    let found = until(&mut pane, |p| matches!(p.read, NodeRead::Found(_))).await;
    assert_eq!((found.node.clone(), label(&found).as_deref()), (Some(node_id("n1")), Some("back")));
}

/// A node's read is dropped when the selection changes, on Refresh and on a context switch: A, then B, then A shows
/// the second read of A, and the first, released last, publishes nothing; a read held at Refresh or at a switch
/// publishes nothing either.
#[tokio::test]
async fn a_node_read_is_dropped_on_a_new_selection_refresh_or_context_switch() {
    let actors = actors().await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held = Fake::Later {
        arrived: arrived.clone(),
        release: release.clone(),
        status: StatusCode::OK,
        body: match detail("a", "stale", &[]) {
            Fake::Answer(_, body) => body,
            _ => unreachable!(),
        },
    };
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = reads.clone();
    let rule: Rule = Arc::new(move |target| match target {
        "/nodes" => Some(nodes_answer(&[node("a", "alive"), node("b", "alive")])),
        "/nodes/a" if counted.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) => Some(held.clone()),
        "/nodes/a" => Some(detail("a", "fresh", &[])),
        "/nodes/b" => Some(detail("b", "b", &[])),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let nodes = model(&server, limits());
    let mut pane = nodes.pane();
    let drop_held = async |nodes: &Nodes, pane: &mut watch::Receiver<NodePane>| {
        release.notify_one();
        for task in nodes.take_aborted() {
            let ended = timeout(WAIT, task).await.expect("the dropped read ends");
            assert!(ended.as_ref().is_err_and(|error| error.is_cancelled()) || ended.is_ok(), "{ended:?}");
        }
        assert!(!pane.has_changed().unwrap(), "the dropped read publishes nothing");
    };

    nodes.select(Some(&node_id("a")));
    timeout(WAIT, arrived.notified()).await.expect("the first read of A is held");
    nodes.take_aborted();
    nodes.select(Some(&node_id("b")));
    until(&mut pane, |p| label(p).as_deref() == Some("b")).await;
    nodes.select(Some(&node_id("a")));
    until(&mut pane, |p| label(p).as_deref() == Some("fresh")).await;
    drop_held(&nodes, &mut pane).await;
    assert_eq!(label(&pane.borrow()).as_deref(), Some("fresh"));

    nodes.select(Some(&node_id("a")));
    timeout(WAIT, arrived.notified()).await.expect("a read of A is held at Refresh");
    nodes.take_aborted();
    nodes.refresh();
    assert_eq!(pane.borrow_and_update().node, None);
    drop_held(&nodes, &mut pane).await;

    // An odd read answers at once: A is selected twice, the second read is held at the switch.
    nodes.select(Some(&node_id("a")));
    until(&mut pane, |p| label(p).as_deref() == Some("fresh")).await;
    nodes.select(Some(&node_id("a")));
    timeout(WAIT, arrived.notified()).await.expect("a read of A is held at the switch");
    nodes.take_aborted();
    nodes.use_client(Some(server.client.clone()));
    assert_eq!(pane.borrow_and_update().node, None);
    drop_held(&nodes, &mut pane).await;
}
