//! The Actors page's model against an in-process server: the paged list, its sample of running turns, the search,
//! and the pane's reads, with answers held back at the server to order them. Real sockets and the real clock, as in
//! the parent module; every wait has a bound.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::prelude::*;
use cicada_test::{TestActors, claim_and_abandon};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Notify, watch};
use tokio::time::timeout;

use super::{Counter, CounterActor, Fake, Rule, Server, Work, config, no_rule, problem, serve};
use crate::actors::{Actors, ListView, Paging, PaneView, Read, RowStatus, Running};
use crate::model::{Limits, Session};

/// The longest a test waits for one change.
const WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Bag {
    id: String,
}

impl ActorRef for Bag {
    const TYPE: &'static str = "bag";
    type Message = Work;
}

#[derive(Debug, Default, Deserialize, JsonSchema, Serialize)]
struct Contents {
    n: u64,
    name: String,
}

struct BagActor;

impl Actor for BagActor {
    type Ref = Bag;
    type State = Contents;

    async fn handle(&self, _: &mut Turn<Self>, _: Work) -> HandlerResult {
        Ok(())
    }
}

async fn test_actors() -> TestActors {
    TestActors::builder().with(|node| node.host(CounterActor).host(BagActor)).build().await.unwrap()
}

fn model(server: &Server, limits: Limits) -> Actors {
    let actors = Actors::new(tokio::runtime::Handle::current(), limits);
    actors.use_client(Some(server.client.clone()));
    actors
}

/// Waits for a list view that `done` accepts.
async fn list_until(list: &mut watch::Receiver<ListView>, done: impl FnMut(&ListView) -> bool) -> ListView {
    timeout(WAIT, list.wait_for(done)).await.expect("the list changes in time").unwrap().clone()
}

/// The list once its paging is idle or failed with every row in.
async fn settled(list: &mut watch::Receiver<ListView>) -> ListView {
    list_until(list, |l| l.paging != Paging::Loading && l.running != Running::Loading).await
}

/// Waits for the pane's current selection to have all its reads answered.
async fn answered(pane: &mut watch::Receiver<PaneView>) -> PaneView {
    let generation = pane.borrow().generation;
    pane_until(pane, generation).await
}

/// Waits for a pane view of `generation` whose reads have all answered.
async fn pane_until(pane: &mut watch::Receiver<PaneView>, generation: u64) -> PaneView {
    let answered = |p: &PaneView| {
        p.generation == generation
            && !matches!(p.actor, Read::Loading)
            && !matches!(p.turn, Read::Loading)
            && !matches!(p.mailbox, Read::Loading)
            && !matches!(p.watching, Read::Loading)
            && !matches!(p.watched_by, Read::Loading)
    };
    timeout(WAIT, pane.wait_for(answered)).await.expect("the pane answers in time").unwrap().clone()
}

/// The server's events so far.
fn events(server: &mut Server) -> Vec<String> {
    std::iter::from_fn(|| server.events.try_recv().ok()).collect()
}

fn count(events: &[String], event: &str) -> usize {
    events.iter().filter(|e| e.as_str() == event).count()
}

/// Scenario: Paging. 250 actors are read in three pages by the server's cursor, none twice; the table asks again for
/// the same page while one is out, and the second ask is dropped.
#[tokio::test]
async fn two_hundred_fifty_actors_are_read_in_three_pages() {
    let actors = test_actors().await;
    for n in 0..250 {
        actors.create(&Counter { id: format!("{n:03}") }, &json!({})).await;
    }
    let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    model.show();
    model.show();
    let first = settled(&mut list).await;
    assert_eq!((first.rows.len(), first.more), (100, true));
    assert!(!first.wants_more(50) && first.wants_more(80));
    for expected in [200, 250] {
        model.more();
        model.more();
        let view = list_until(&mut list, |l| l.rows.len() == expected && l.paging == Paging::Idle).await;
        assert_eq!(view.more, expected < 250);
    }
    model.more();
    let rows = list.borrow().rows.clone();
    let addresses: std::collections::BTreeSet<_> = rows.iter().map(|row| row.address.to_string()).collect();
    assert_eq!(addresses.len(), 250);
    assert_eq!(rows[0].address.as_str(), "counter/000");
    assert_eq!((rows[0].type_name.as_str(), rows[0].version, rows[0].paused), ("counter", 0, false));
    let events = events(&mut server);
    let pages: Vec<_> = events.iter().filter(|e| e.starts_with("served /actors/states")).collect();
    assert_eq!(pages.len(), 3, "{pages:?}");
    assert_eq!(count(&events, "served /turns?limit=200"), 1, "one sample per reload");
}

/// The cursor of a page is passed back unchanged; one page is out at a time, so more asks while it is out are
/// dropped.
#[tokio::test]
async fn the_next_page_uses_the_servers_cursor_and_one_page_is_out_at_a_time() {
    let actors = test_actors().await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let row = |id: &str| {
        json!({"address": format!("counter/{id}"), "type": "counter", "state": {"big": "x"}, "version": 3,
               "createdAt": "2026-11-27T10:00:00.000Z", "paused": true})
    };
    let first = json!({"items": [row("a"), row("b")], "next": "opaque-Cursor_1.x"});
    let second = Fake::Later {
        arrived: arrived.clone(),
        release: release.clone(),
        status: StatusCode::OK,
        body: json!({"items": [row("c")], "next": null}),
    };
    let rule: Rule = Arc::new(move |target| match target {
        "/actors/states?limit=100" => Some(Fake::Answer(StatusCode::OK, first.clone())),
        "/actors/states?cursor=opaque-Cursor_1.x&limit=100" => Some(second.clone()),
        _ => None,
    });
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    model.show();
    let view = settled(&mut list).await;
    assert_eq!(view.rows.len(), 2);
    assert_eq!(view.status(&view.rows[0]), RowStatus { running: false, paused: true });
    model.more();
    timeout(WAIT, arrived.notified()).await.expect("the second page is asked for");
    model.more();
    model.more();
    assert_eq!(list.borrow().paging, Paging::Loading);
    release.notify_waiters();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 3).await;
    assert!(!view.more && !view.wants_more(2));
    let events = events(&mut server);
    assert_eq!(count(&events, "arrived /actors/states?cursor=opaque-Cursor_1.x&limit=100"), 1, "{events:?}");
}

/// A page that fails keeps its cursor: scrolling does not ask again, Retry does, from the same cursor.
#[tokio::test]
async fn a_failed_page_is_asked_again_only_by_retry() {
    let actors = test_actors().await;
    for n in 0..150 {
        actors.create(&Counter { id: format!("{n:03}") }, &json!({})).await;
    }
    let fails = Arc::new(AtomicUsize::new(1));
    let failing = fails.clone();
    let rule: Rule = Arc::new(move |target| {
        let next_page = target.starts_with("/actors/states?cursor=");
        (next_page && failing.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok())
            .then(|| problem(500))
    });
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    model.show();
    settled(&mut list).await;
    model.more();
    let failed = list_until(&mut list, |l| matches!(l.paging, Paging::Failed(_))).await;
    let Paging::Failed(error) = &failed.paging else { unreachable!() };
    assert!(error.contains("500"), "{error}");
    assert_eq!(failed.rows.len(), 100);
    model.more();
    assert!(matches!(list.borrow().paging, Paging::Failed(_)), "scrolling does not ask again");
    model.retry();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 150).await;
    assert!(!view.more);
    let pages: Vec<_> =
        events(&mut server).into_iter().filter(|e| e.starts_with("served /actors/states?cursor=")).collect();
    assert_eq!(pages.len(), 2, "{pages:?}");
    assert_eq!(pages[0], pages[1], "Retry asks from the same cursor");
}

/// The status column: running from one sample of the running turns, paused from the row, both together when both
/// hold; the sample is read once per reload, not per page.
#[tokio::test]
async fn running_and_paused_come_from_one_sample_and_the_row() {
    let actors = test_actors().await;
    for id in ["idle", "paused", "running", "running-paused"] {
        actors.create(&Counter { id: id.into() }, &json!({})).await;
    }
    for id in ["running", "running-paused"] {
        actors.send(&Counter { id: id.into() }, Work::Ok).await;
    }
    let lease = Duration::from_secs(600);
    let claimed = claim_and_abandon(actors.store(), &actors.system().hosted_types(), lease, 2).await.unwrap();
    assert_eq!(claimed.len(), 2);
    let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
    for id in ["paused", "running-paused"] {
        let address = ActorAddress::parse(&format!("counter/{id}")).unwrap();
        assert!(server.client.pause_mailbox(&address).await.unwrap());
    }
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    model.show();
    let view = settled(&mut list).await;
    let status: Vec<_> = view.rows.iter().map(|row| (row.address.key().to_owned(), view.status(row).text())).collect();
    let expected = [("idle", ""), ("paused", "paused"), ("running", "running"), ("running-paused", "running · paused")];
    assert_eq!(status, expected.map(|(key, text)| (key.to_owned(), text)));
    assert_eq!(view.footer(), None);
    model.refresh();
    settled(&mut list).await;
    let events = events(&mut server);
    assert_eq!(count(&events, "served /turns?limit=200"), 2, "one sample per reload: {events:?}");
}

/// The footer says what the running status covers: the first 200 running turns when the server has more, and
/// unknown, with no row marked running, when the sample fails.
#[tokio::test]
async fn the_footer_names_a_partial_or_failed_sample() {
    let actors = test_actors().await;
    actors.create(&Counter { id: "a".into() }, &json!({})).await;
    let turn = json!({"actor": "counter/a", "messageId": "m1", "node": "n1",
                      "leaseUntil": "2026-11-27T10:00:00.000Z", "cancelRequested": false});
    let page = json!({"items": [turn], "next": "more"});
    let rule: Rule = Arc::new(move |target| match target {
        "/turns?limit=200" => Some(Fake::Answer(StatusCode::OK, page.clone())),
        "/turns?prefix=counter%2Fa&limit=200" => Some(problem(503)),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits { debounce: Duration::from_millis(10), ..Limits::DEFAULT });
    let mut list = model.list();
    model.show();
    let view = settled(&mut list).await;
    assert!(view.status(&view.rows[0]).running);
    assert_eq!(view.footer().as_deref(), Some("The running status covers the first 200 running turns."));
    model.search("counter/a");
    let view = list_until(&mut list, |l| l.prefix == "counter/a" && matches!(l.running, Running::Unknown(_))).await;
    let view = if view.paging == Paging::Loading { settled(&mut list).await } else { view };
    assert_eq!(view.rows.len(), 1);
    assert!(!view.status(&view.rows[0]).running, "no row is marked running");
    let footer = view.footer().unwrap();
    assert!(footer.starts_with("The running status is unknown: ") && footer.contains("503"), "{footer}");
}

/// Scenario: A search drops older pages. The first page of the empty prefix is held at the server when the user types;
/// it is dropped at the first key press, and only the last prefix is read, once the field is still.
#[tokio::test]
async fn a_search_drops_the_pages_of_the_earlier_prefix() {
    let actors = test_actors().await;
    for n in 0..3 {
        actors.create(&Counter { id: format!("{n}") }, &json!({})).await;
        actors.create(&Bag { id: format!("{n}") }, &Contents::default()).await;
    }
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let stall = Fake::Wait { arrived: arrived.clone(), release: release.clone() };
    let rule: Rule = Arc::new(move |target| (target == "/actors/states?limit=100").then(|| stall.clone()));
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    // The keys come in one synchronous run, far within the second the field must be still.
    let model = model(&server, Limits { debounce: Duration::from_secs(1), ..Limits::DEFAULT });
    let mut list = model.list();
    model.show();
    timeout(WAIT, arrived.notified()).await.expect("the first page is held at the server");
    let before = list.borrow_and_update().generation;
    model.search("b");
    assert!(list.has_changed().unwrap(), "the first key press replaces the rows at once");
    let typed = list.borrow_and_update().clone();
    assert!(typed.generation > before && typed.rows.is_empty() && typed.paging == Paging::Loading);
    model.search("ba");
    model.search("bag/");
    release.notify_waiters();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && !l.rows.is_empty()).await;
    assert_eq!(view.prefix, "bag/");
    let rows: Vec<_> = view.rows.iter().map(|row| row.address.to_string()).collect();
    assert_eq!(rows, ["bag/0", "bag/1", "bag/2"]);
    let events = events(&mut server);
    let pages: Vec<_> = events.iter().filter(|e| e.starts_with("served /actors/states")).collect();
    assert!(pages.iter().all(|e| !e.contains("prefix=b&") && !e.contains("prefix=ba&")), "{pages:?}");
    assert_eq!(count(&events, "served /actors/states?prefix=bag%2F&limit=100"), 1, "{events:?}");
    let aborted = model.take_aborted();
    assert!(!aborted.is_empty());
    for task in aborted {
        let ended = timeout(WAIT, task).await.expect("an aborted read ends");
        assert!(ended.as_ref().is_err_and(|e| e.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    assert!(!list.has_changed().unwrap(), "the held page of the earlier prefix is dropped");
}

/// A search still waiting for the field to settle is cancelled when the window closes: its task ends cancelled, and
/// nothing is read for its prefix.
#[tokio::test]
async fn a_pending_search_is_cancelled_at_close() {
    let actors = test_actors().await;
    let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
    // A wait no test outlasts: the task can end only by being cancelled.
    let model = model(&server, Limits { debounce: Duration::from_secs(3600), ..Limits::DEFAULT });
    model.search("counter/");
    model.stop();
    let aborted = model.take_aborted();
    assert_eq!(aborted.len(), 1, "the pending search");
    for task in aborted {
        let ended = timeout(WAIT, task).await.expect("the pending search ends");
        assert!(ended.is_err_and(|e| e.is_cancelled()));
    }
    assert!(events(&mut server).iter().all(|e| !e.contains("prefix=counter")));
}

/// A page held at the server of one context when the user switches to another is dropped; the new context's list is
/// read when the page is shown.
#[tokio::test]
async fn a_page_in_flight_at_a_context_switch_is_dropped() {
    let (slow_actors, fast_actors) = (test_actors().await, test_actors().await);
    slow_actors.create(&Counter { id: "old".into() }, &json!({})).await;
    fast_actors.create(&Counter { id: "new".into() }, &json!({})).await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let stall = Fake::Wait { arrived: arrived.clone(), release: release.clone() };
    let slow_rule: Rule = Arc::new(move |t| t.starts_with("/actors/states").then(|| stall.clone()));
    let slow = serve(&slow_actors, slow_rule, Duration::ZERO).await;
    let fast = serve(&fast_actors, no_rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let env = config(&dir, &[("slow", &slow.url), ("fast", &fast.url)]);
    let session = Session::new(env, tokio::runtime::Handle::current(), Limits::DEFAULT);
    session.use_context(None);
    let mut list = session.actors().list();
    let mut pane = session.actors().pane();
    session.actors().show();
    session.actors().select(Some(ActorAddress::parse("counter/old").unwrap()));
    timeout(WAIT, arrived.notified()).await.expect("the old page is held");

    session.use_context(Some("fast".into()));
    let cleared = list.borrow_and_update().clone();
    assert!(cleared.rows.is_empty() && cleared.paging == Paging::Idle, "{cleared:?}");
    assert_eq!(pane.borrow_and_update().address, None, "the selection goes with the context");
    session.actors().show();
    let view = settled(&mut list).await;
    assert_eq!(view.rows.iter().map(|r| r.address.to_string()).collect::<Vec<_>>(), ["counter/new"]);
    release.notify_waiters();
    for task in session.actors().take_aborted() {
        let ended = timeout(WAIT, task).await.expect("an aborted read ends");
        assert!(ended.as_ref().is_err_and(|e| e.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    assert!(!list.has_changed().unwrap(), "the old page is dropped");
    session.stop();
}

/// The watches of the scenario's actor: one where it watches, one where it is the exact target.
fn watches_rule(rule: impl Fn(&str) -> Option<Fake> + Send + Sync + 'static) -> Rule {
    Arc::new(move |target| match target {
        "/watches?limit=50&watcher=bag%2Fa" => Some(Fake::Answer(
            StatusCode::OK,
            json!({"items": [{"target": "counter/x", "watcher": "bag/a"}], "next": null}),
        )),
        "/watches?limit=50&target=bag%2Fa" => Some(Fake::Answer(
            StatusCode::OK,
            json!({"items": [{"target": "bag/a", "watcher": "bag/w1"}, {"target": "bag/a", "watcher": "bag/w2"}],
                   "next": "more"}),
        )),
        other => rule(other),
    })
}

/// Scenario: The pane. An actor with a running turn and two pending messages, paused as well: its state as JSON, the
/// turn's type and attempt from the mailbox's first page, the two pending messages, and both kinds of watches apart.
#[tokio::test]
async fn the_pane_shows_the_state_turn_mailbox_and_watches() {
    let actors = test_actors().await;
    let bag = Bag { id: "a".into() };
    actors.create(&bag, &Contents { n: 7, name: "zoë".into() }).await;
    let running = actors.send(&bag, Work::Ok).await;
    let pending = [actors.send(&bag, Work::Fail).await, actors.send(&bag, Work::Ok).await];
    let claimed =
        claim_and_abandon(actors.store(), &actors.system().hosted_types(), Duration::from_secs(600), 1).await.unwrap();
    assert_eq!(claimed[0].message_id, running.id);
    let server = serve(&actors, watches_rule(|_| None), Duration::ZERO).await;
    let address = ActorAddress::parse("bag/a").unwrap();
    assert!(server.client.pause_mailbox(&address).await.unwrap());
    let model = model(&server, Limits::DEFAULT);
    let mut pane = model.pane();
    model.select(Some(address.clone()));
    let generation = pane.borrow().generation;
    let view = pane_until(&mut pane, generation).await;

    assert_eq!(view.address.as_ref(), Some(&address));
    let Read::Ready(Some(details)) = &view.actor else { panic!("{:?}", view.actor) };
    assert_eq!(details.state.text, "{\n  \"n\": 7,\n  \"name\": \"zoë\"\n}");
    assert!(!details.state.cut && details.paused && details.parent.is_none() && details.pinned_to.is_none());
    assert_eq!((details.type_name.as_str(), details.version), ("bag", 0));
    let Read::Ready(Some(turn)) = &view.turn else { panic!("{:?}", view.turn) };
    assert_eq!(turn.message_id, running.id);
    let message = view.turn_message().expect("the turn's message is on the first page");
    assert_eq!((message.message_type.as_str(), message.attempt, message.running), ("Ok", 1, true));
    let pending_ids: Vec<_> = view.pending().map(|row| row.id.clone()).collect();
    assert_eq!(pending_ids, pending.map(|m| m.id));
    assert_eq!(view.pending().map(|row| row.message_type.as_str()).collect::<Vec<_>>(), ["Fail", "Ok"]);
    let (Read::Ready(watching), Read::Ready(watched_by)) = (&view.watching, &view.watched_by) else {
        panic!("{view:?}")
    };
    assert_eq!(watching.items.iter().map(|w| w.target.to_string()).collect::<Vec<_>>(), ["counter/x"]);
    assert!(!watching.more);
    assert_eq!(watched_by.items.iter().map(|w| w.watcher.to_string()).collect::<Vec<_>>(), ["bag/w1", "bag/w2"]);
    assert!(watched_by.more);
    assert!(!view.not_found());
}

/// Without its message on the mailbox's first page, the turn shows without a type and attempt; an idle actor shows
/// no turn, which is not an error.
#[tokio::test]
async fn a_turn_off_the_first_page_has_no_type_and_an_idle_actor_no_turn() {
    let actors = test_actors().await;
    actors.create(&Bag { id: "a".into() }, &Contents::default()).await;
    actors.create(&Bag { id: "idle".into() }, &Contents::default()).await;
    let turn = json!({"actor": "bag/a", "messageId": "elsewhere", "node": "n1",
                      "leaseUntil": "2026-11-27T10:00:00.000Z", "cancelRequested": true});
    let rule = watches_rule(move |target| {
        (target == "/actors/bag/a/turn").then(|| Fake::Answer(StatusCode::OK, turn.clone()))
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut pane = model.pane();
    model.select(Some(ActorAddress::parse("bag/a").unwrap()));
    let view = answered(&mut pane).await;
    let Read::Ready(Some(turn)) = &view.turn else { panic!("{:?}", view.turn) };
    assert!(turn.cancel_requested);
    assert_eq!(view.turn_message(), None);
    assert_eq!(view.mailbox, Read::Ready(Default::default()));

    model.select(Some(ActorAddress::parse("bag/idle").unwrap()));
    let view = answered(&mut pane).await;
    assert_eq!(view.turn, Read::Ready(None));
    assert!(matches!(view.actor, Read::Ready(Some(_))));
}

/// An actor that is gone shows "not found"; one failing read shows its error alone.
#[tokio::test]
async fn a_missing_actor_is_not_found_and_one_read_fails_alone() {
    let actors = test_actors().await;
    actors.create(&Bag { id: "a".into() }, &Contents::default()).await;
    let rule = watches_rule(|target| (target == "/actors/bag/a/mailbox?limit=50").then(|| problem(500)));
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut pane = model.pane();

    model.select(Some(ActorAddress::parse("bag/ghost").unwrap()));
    let view = answered(&mut pane).await;
    assert!(view.not_found(), "{view:?}");

    model.select(Some(ActorAddress::parse("bag/a").unwrap()));
    let view = answered(&mut pane).await;
    assert!(matches!(&view.mailbox, Read::Failed(error) if error.contains("500")), "{:?}", view.mailbox);
    assert!(matches!(view.actor, Read::Ready(Some(_))) && view.turn == Read::Ready(None));
    assert!(matches!(view.watching, Read::Ready(_)) && matches!(view.watched_by, Read::Ready(_)));

    model.select(None);
    let empty = pane.borrow().clone();
    assert_eq!(empty.address, None);
}

/// Scenario: Back to an earlier selection. A, then B, then A again before the first read of A answers: the pane shows
/// only the last read of A, also once the first is answered.
#[tokio::test]
async fn a_then_b_then_a_shows_only_the_last_read_of_a() {
    let actors = test_actors().await;
    actors.create(&Bag { id: "a".into() }, &Contents { n: 2, name: "last".into() }).await;
    actors.create(&Bag { id: "b".into() }, &Contents { n: 1, name: "b".into() }).await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let stale = json!({"address": "bag/a", "type": "bag", "state": {"n": 1, "name": "first"}, "version": 0,
                       "createdAt": "2026-11-27T10:00:00.000Z"});
    let first_read =
        Fake::Later { arrived: arrived.clone(), release: release.clone(), status: StatusCode::OK, body: stale };
    let reads = Arc::new(AtomicUsize::new(0));
    let rule = watches_rule(move |target| {
        (target == "/actors/bag/a" && reads.fetch_add(1, Ordering::SeqCst) == 0).then(|| first_read.clone())
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut pane = model.pane();
    let (a, b) = (ActorAddress::parse("bag/a").unwrap(), ActorAddress::parse("bag/b").unwrap());

    model.select(Some(a.clone()));
    timeout(WAIT, arrived.notified()).await.expect("the first read of A is held");
    model.select(Some(b));
    model.select(Some(a.clone()));
    let generation = pane.borrow().generation;
    let view = pane_until(&mut pane, generation).await;
    let Read::Ready(Some(details)) = &view.actor else { panic!("{:?}", view.actor) };
    assert!(details.state.text.contains("\"last\""), "{}", details.state.text);

    // The held answer is let go; the read that waited for it was cancelled at the selection of B, and once it has
    // ended it can publish nothing.
    release.notify_waiters();
    for task in model.take_aborted() {
        let ended = timeout(WAIT, task).await.expect("an aborted read ends");
        assert!(ended.as_ref().is_err_and(|e| e.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    let view = pane.borrow().clone();
    assert_eq!(view.generation, generation);
    let Read::Ready(Some(details)) = &view.actor else { panic!("{:?}", view.actor) };
    assert!(details.state.text.contains("\"last\""), "the first read of A is dropped: {}", details.state.text);
}
