//! The Running Turns page's model against an in-process server: the paged list by the server's cursor, the pane from
//! the row, Show Actor, and back to a turn that ended or moved on. Real sockets and the real clock, as in the parent
//! module, but no outcome depends on the machine's speed: a search reads at once, a read's deadline is far past every
//! wait, the store's clock does not move, so a claimed lease never runs out during a test, and every wait has a bound.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::prelude::*;
use cicada_test::claim_and_abandon;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::time::timeout;

use super::{Counter, Fake, Rule, Server, Work, actors, config, no_rule, problem, serve};
use crate::model::{Limits, Navigation, Section, Session};
use crate::pager::Paging;
use crate::turns::{RunningTurns, TurnPane, TurnState, TurnsView};

/// The longest a test waits for one change.
const WAIT: Duration = Duration::from_secs(10);
/// A lease far longer than any test: the store's clock does not move by itself.
const LEASE: Duration = Duration::from_secs(3600);

/// Searches read at once, and a read times out only well after a wait would have failed.
fn limits() -> Limits {
    Limits { debounce: Duration::ZERO, deadline: 6 * WAIT, ..Limits::DEFAULT }
}

fn model(server: &Server) -> RunningTurns {
    let turns = RunningTurns::new(tokio::runtime::Handle::current(), limits());
    turns.use_client(Some(server.client.clone()));
    turns
}

fn session(server: &Server, dir: &tempfile::TempDir) -> Session {
    let env = config(dir, &[("prod", &server.url)]);
    let session = Session::new(env, tokio::runtime::Handle::current(), limits());
    session.use_context(None);
    session
}

async fn until<T: Clone>(view: &mut watch::Receiver<T>, done: impl FnMut(&T) -> bool) -> T {
    timeout(WAIT, view.wait_for(done)).await.expect("the model changes in time").unwrap().clone()
}

/// A served running turn.
fn turn(actor: &str, message: &str) -> Value {
    json!({"actor": actor, "messageId": message, "node": "n1", "leaseUntil": "2026-11-27T10:00:00.000Z",
           "cancelRequested": false})
}

fn page(items: &[Value], next: Option<&str>) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"items": items, "next": next}))
}

fn no_turn() -> Fake {
    let body = json!({"type": "urn:cicada:problem:no-running-turn", "title": "Not Found", "status": 404});
    Fake::Answer(StatusCode::NOT_FOUND, body)
}

fn routes(routes: Vec<(&'static str, Fake)>) -> Rule {
    Arc::new(move |target| routes.iter().find(|(path, _)| *path == target).map(|(_, fake)| fake.clone()))
}

fn actors_of(view: &TurnsView) -> Vec<String> {
    view.rows.iter().map(|row| row.actor.to_string()).collect()
}

fn served(server: &mut Server, prefix: &str) -> Vec<String> {
    std::iter::from_fn(|| server.events.try_recv().ok()).filter(|e| e.starts_with(prefix)).collect()
}

fn address(text: &str) -> ActorAddress {
    ActorAddress::parse(text).unwrap()
}

fn select_turns(navigation: &mut Navigation) -> bool {
    let before = navigation.current();
    navigation.select(Section::RunningTurns);
    navigation.current() != before
}

/// Running Turns lists 150 claimed turns in two pages by the server's cursor, with their actor, message, node and
/// lease end. Between the pages two leases begin: one of an actor before the cursor, which the second page does not
/// hold, and one after it, which it does; every actor shows once.
#[tokio::test]
async fn running_turns_page_by_the_cursor_while_leases_begin_between_pages() {
    let actors = actors().await;
    for n in 0..150 {
        let counter = Counter { id: format!("{n:03}") };
        actors.create(&counter, &json!({})).await;
        actors.send(&counter, Work::Ok).await;
    }
    let types = actors.system().hosted_types();
    let claimed = claim_and_abandon(actors.store(), &types, LEASE, 150).await.unwrap();
    assert_eq!(claimed.len(), 150);
    let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
    let model = model(&server);
    let mut list = model.list();
    model.search("counter/");
    let first = until(&mut list, |l| l.paging == Paging::Idle && !l.rows.is_empty()).await;
    assert_eq!((first.rows.len(), first.more), (100, true));
    let row = &first.rows[0];
    assert_eq!(row.actor.as_str(), "counter/000");
    assert_eq!(row.message_id, claimed.iter().find(|c| c.address == row.actor).unwrap().message_id);
    assert!(!row.cancel_requested);

    for id in ["050x", "120x"] {
        let counter = Counter { id: id.into() };
        actors.create(&counter, &json!({})).await;
        actors.send(&counter, Work::Ok).await;
    }
    assert_eq!(claim_and_abandon(actors.store(), &types, LEASE, 2).await.unwrap().len(), 2);
    model.more();
    let view = until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
    let listed = actors_of(&view);
    assert_eq!(listed.len(), 151);
    assert!(listed.contains(&"counter/120x".to_owned()) && !listed.contains(&"counter/050x".to_owned()));
    let mut once = listed.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), listed.len(), "every actor shows once");
    assert!(Arc::ptr_eq(&first.rows[0], &view.rows[0]), "a later view shares the rows read before");
    let asked = served(&mut server, "served /turns");
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(asked[1].contains("cursor="), "{asked:?}");
}

/// The cursor goes back as the server gave it, also when the turn of the first page has ended and others began by
/// the next: the rows are appended as they come, with no read of the ended one. A page with no rows but a next cursor
/// is followed at once.
#[tokio::test]
async fn the_cursor_goes_back_unchanged_while_leases_end_between_pages() {
    let actors = actors().await;
    let rule = routes(vec![
        ("/turns?limit=100", page(&[turn("bag/a", "m1"), turn("bag/b", "m2")], Some("bag/b?x=1&y"))),
        ("/turns?cursor=bag%2Fb%3Fx%3D1%26y&limit=100", page(&[], Some("c2"))),
        ("/turns?cursor=c2&limit=100", page(&[turn("bag/d", "m4")], None)),
    ]);
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server);
    let mut list = model.list();
    model.show();
    let first = until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    assert!(first.wants_more(0));
    model.more();
    let view = until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
    assert_eq!(actors_of(&view), ["bag/a", "bag/b", "bag/d"]);
    assert_eq!(
        served(&mut server, "served /turns"),
        [
            "served /turns?limit=100",
            "served /turns?cursor=bag%2Fb%3Fx%3D1%26y&limit=100",
            "served /turns?cursor=c2&limit=100"
        ]
    );
}

/// Selecting a turn fills the pane from its row, with no read; Refresh clears it, and a search drops a turn outside
/// the prefix. A failed page shows its error, and Retry reads it again from the same cursor.
#[tokio::test]
async fn the_pane_comes_from_the_row_and_a_failed_page_is_retried() {
    let actors = actors().await;
    let fails = Arc::new(AtomicUsize::new(1));
    let failing = fails.clone();
    let rule: Rule = Arc::new(move |target| match target {
        "/turns?limit=100" if failing.fetch_sub(1, Ordering::SeqCst) == 1 => Some(problem(500)),
        "/turns?limit=100" => Some(page(&[turn("bag/a", "m1"), turn("sync/b", "m2")], None)),
        _ => None,
    });
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server);
    let (mut list, mut pane) = (model.list(), model.pane());
    model.show();
    let failed = until(&mut list, |l| matches!(l.paging, Paging::Failed(_))).await;
    assert!(matches!(&failed.paging, Paging::Failed(error) if error.contains("500")), "{:?}", failed.paging);
    model.retry();
    let view = until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;

    model.select(Some(&address("sync/b")));
    let shown = pane.borrow_and_update().clone();
    assert!(shown.turn.as_ref().is_some_and(|t| Arc::ptr_eq(t, &view.rows[1])));
    assert_eq!((shown.state, model.selected()), (TurnState::Running, Some(address("sync/b"))));
    assert!(served(&mut server, "served /actors").is_empty(), "the pane reads nothing");
    model.search("bag/");
    assert_eq!((pane.borrow().turn.clone(), model.selected()), (None, None));
    until(&mut list, |l| l.prefix == "bag/" && l.paging == Paging::Idle).await;
    model.search("");
    until(&mut list, |l| l.prefix.is_empty() && l.paging == Paging::Idle && l.rows.len() == 2).await;
    model.select(Some(&address("bag/a")));
    model.refresh();
    assert_eq!((pane.borrow().turn.clone(), model.selected()), (None, None));
}

/// Scenario: Back to an ended turn. Show Actor from a turn selects its actor on the Actors page. Back reads the
/// actor's turn fresh: once it has ended, the pane shows the turn as it was read, marked "no longer running", with no
/// row selected; once the actor runs another message, the pane shows that turn with its row selected. A failed read
/// shows its error, and Retry reads it again.
#[tokio::test]
async fn back_to_a_turn_reads_it_fresh_and_tells_ended_from_moved() {
    let actors = actors().await;
    for id in ["a", "b"] {
        actors.create(&Counter { id: id.into() }, &json!({})).await;
    }
    let mode = Arc::new(AtomicUsize::new(0));
    let rule_mode = mode.clone();
    let rule: Rule = Arc::new(move |target| match target {
        "/turns?prefix=counter%2F&limit=100" => Some(page(&[turn("counter/a", "m1"), turn("counter/b", "m2")], None)),
        "/actors/counter/a/turn" => Some(match rule_mode.load(Ordering::SeqCst) {
            0 => no_turn(),
            1 => Fake::Answer(StatusCode::OK, turn("counter/a", "m9")),
            _ => problem(500),
        }),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let mut navigation = Navigation::default();
    let turns = session.turns();
    let (mut list, mut pane) = (turns.list(), turns.pane());
    session.navigate(&mut navigation, select_turns);
    turns.search("counter/");
    until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    turns.select(Some(&address("counter/a")));

    let mut actor_pane = session.actors().pane();
    session.show_actor(&mut navigation, address("counter/a"));
    assert_eq!(navigation.current(), Section::Actors);
    until(&mut actor_pane, |p| p.address == Some(address("counter/a")) && !p.not_found()).await;

    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!(navigation.current(), Section::RunningTurns);
    let ended = until(&mut pane, |p| p.state == TurnState::Ended).await;
    assert_eq!(ended.turn.as_ref().map(|t| t.message_id.to_string()), Some("m1".into()), "the turn as it was read");
    assert_eq!(turns.selected(), None);

    mode.store(2, Ordering::SeqCst);
    assert!(session.navigate(&mut navigation, Navigation::forward));
    assert!(session.navigate(&mut navigation, Navigation::back));
    let failed = until(&mut pane, |p| matches!(p.state, TurnState::Failed(_))).await;
    assert!(matches!(&failed.state, TurnState::Failed(error) if error.contains("500")), "{:?}", failed.state);
    mode.store(1, Ordering::SeqCst);
    turns.retry_check();
    let moved = until(&mut pane, |p: &TurnPane| p.state == TurnState::Running).await;
    assert_eq!(moved.turn.as_ref().map(|t| t.message_id.to_string()), Some("m9".into()), "the turn as it is now");
    assert_eq!(turns.selected(), Some(address("counter/a")));
    session.stop();
}

/// Back to a place whose prefix the page no longer shows reads that prefix again and the turn fresh: the turn's row
/// is selected once a page brings it.
#[tokio::test]
async fn back_to_another_prefix_selects_the_row_once_a_page_brings_it() {
    let actors = actors().await;
    actors.create(&Counter { id: "a".into() }, &json!({})).await;
    let rule = routes(vec![
        ("/turns?prefix=counter%2F&limit=100", page(&[turn("counter/a", "m1")], None)),
        ("/turns?prefix=sync%2F&limit=100", page(&[], None)),
        ("/actors/counter/a/turn", Fake::Answer(StatusCode::OK, turn("counter/a", "m1"))),
    ]);
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let mut navigation = Navigation::default();
    let turns = session.turns();
    let (mut list, mut pane) = (turns.list(), turns.pane());
    session.navigate(&mut navigation, select_turns);
    turns.search("counter/");
    until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
    turns.select(Some(&address("counter/a")));
    session.show_actor(&mut navigation, address("counter/a"));
    session.navigate(&mut navigation, select_turns);
    turns.search("sync/");
    until(&mut list, |l| l.prefix == "sync/" && l.paging == Paging::Idle).await;
    assert!(session.navigate(&mut navigation, Navigation::back));
    assert!(session.navigate(&mut navigation, Navigation::back));

    let view = until(&mut list, |l| l.prefix == "counter/" && l.paging == Paging::Idle && l.rows.len() == 1).await;
    until(&mut pane, |p| p.state == TurnState::Running && p.turn.is_some()).await;
    // The read and the page answer in either order, each under the lock: once both did, the row is selected.
    assert_eq!(turns.selected(), Some(address("counter/a")));
    assert_eq!(view.index(&address("counter/a")), Some(0));
    session.stop();
}
