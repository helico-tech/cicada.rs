//! The Messages page's model against an in-process server: the paged list of pending messages, each once, the pane
//! filled from the selected row, and Show Actor with its history. Real sockets and the real clock, as in the parent
//! module, but no outcome depends on the machine's speed: a search reads at once (no debounce), a read's deadline is
//! far past every wait, answers and formattings that must come in an order are held, and every wait has a bound.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::prelude::*;
use cicada::store::MessageId;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use tokio::time::timeout;

use super::{Counter, Fake, Rule, Server, Work, actors, config, no_rule, problem, serve};
use crate::actors::{ActorPath, ListView, PaneView};
use crate::messages::{MessagePane, Messages, MessagesView};
use crate::model::{Limits, Navigation, Section, Session};
use crate::pager::Paging;

/// The longest a test waits for one change.
const WAIT: Duration = Duration::from_secs(10);

/// Searches read at once, and a read times out only well after a wait would have failed.
fn limits() -> Limits {
    Limits { debounce: Duration::ZERO, deadline: 6 * WAIT, ..Limits::DEFAULT }
}

pub(super) fn model(server: &Server, limits: Limits) -> Messages {
    let messages = Messages::new(tokio::runtime::Handle::current(), limits);
    messages.use_client(Some(server.client.clone()));
    messages
}

pub(super) async fn list_until(
    list: &mut watch::Receiver<MessagesView>,
    done: impl FnMut(&MessagesView) -> bool,
) -> MessagesView {
    timeout(WAIT, list.wait_for(done)).await.expect("the list changes in time").unwrap().clone()
}

pub(super) async fn pane_until(
    pane: &mut watch::Receiver<MessagePane>,
    done: impl FnMut(&MessagePane) -> bool,
) -> MessagePane {
    timeout(WAIT, pane.wait_for(done)).await.expect("the pane changes in time").unwrap().clone()
}

/// A served envelope.
pub(super) fn envelope(id: &str, to: &str, attempt: u32) -> Value {
    json!({"id": id, "to": to, "from": "bag/sender", "type": "Ok", "payload": {"n": attempt, "name": "zoë"},
           "deliverAt": "2026-11-27T10:00:00.000Z", "correlationId": "order-7", "headers": {"trace": "t-1"},
           "seq": 1, "attempt": attempt, "failures": 1})
}

pub(super) fn id(id: &str) -> MessageId {
    MessageId::try_from(id.to_owned()).unwrap()
}

fn page(items: &[Value], next: Option<&str>) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"items": items, "next": next}))
}

fn ids(view: &MessagesView) -> Vec<String> {
    view.rows.iter().map(|row| row.id.to_string()).collect()
}

fn served(server: &mut Server, prefix: &str) -> Vec<String> {
    std::iter::from_fn(|| server.events.try_recv().ok()).filter(|e| e.starts_with(prefix)).collect()
}

/// Scenario: A pending message, paged. 150 messages are read in two pages by the server's cursor, with their id,
/// target, type, due time and attempt; the rows of the first page are shared with the later view, not copied.
#[tokio::test]
async fn one_hundred_fifty_messages_are_read_in_two_pages() {
    let actors = actors().await;
    let mut sent = Vec::new();
    for n in 0..150 {
        let counter = Counter { id: format!("{n:03}") };
        actors.create(&counter, &json!({})).await;
        sent.push(actors.send(&counter, Work::Ok).await);
    }
    let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
    let model = model(&server, limits());
    let mut list = model.list();
    model.search("counter/");
    let first = list_until(&mut list, |l| l.paging == Paging::Idle && !l.rows.is_empty()).await;
    assert_eq!((first.rows.len(), first.more, first.prefix.as_str()), (100, true, "counter/"));
    assert!(!first.wants_more(50) && first.wants_more(80));
    model.more();
    model.more();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 150).await;
    assert!(!view.more);
    assert!(Arc::ptr_eq(&first.rows[0], &view.rows[0]), "a later view shares the rows read before");
    assert_eq!(ids(&view), sent.iter().map(|m| m.id.to_string()).collect::<Vec<_>>());
    let row = &view.rows[0];
    assert_eq!((row.to.as_str(), row.message_type.as_str(), row.attempt), ("counter/000", "Ok", 0));
    assert_eq!(row.deliver_at, sent[0].deliver_at);
    let pages = served(&mut server, "served /messages");
    assert_eq!(pages.len(), 2, "{pages:?}");
    assert_eq!(pages[0], "served /messages?prefix=counter%2F&limit=100");
    assert!(pages[1].starts_with("served /messages?prefix=counter%2F&cursor="), "{pages:?}");
}

/// Scenario: Duplicates across pages. A message retried between pages is on two of them: the first row is kept and
/// the later one dropped. A page of only duplicates leaves the rows as they were, yet its cursor is used: the next
/// page is asked for at once, without the view asking again.
#[tokio::test]
async fn a_message_on_two_pages_is_shown_once_and_a_page_of_duplicates_still_advances() {
    let actors = actors().await;
    let pages = [
        ("/messages?limit=100", page(&[envelope("m1", "bag/a", 1), envelope("m2", "bag/b", 1)], Some("c1"))),
        ("/messages?cursor=c1&limit=100", page(&[envelope("m2", "bag/b", 2), envelope("m3", "bag/c", 1)], Some("c2"))),
        ("/messages?cursor=c2&limit=100", page(&[envelope("m3", "bag/c", 2), envelope("m3", "bag/c", 3)], Some("c3"))),
        ("/messages?cursor=c3&limit=100", page(&[envelope("m4", "bag/d", 1)], None)),
    ];
    let rule: Rule =
        Arc::new(move |target| pages.iter().find(|(path, _)| *path == target).map(|(_, fake)| fake.clone()));
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, limits());
    let mut list = model.list();
    model.show();
    list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    model.more();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 3).await;
    assert_eq!(ids(&view), ["m1", "m2", "m3"]);
    assert_eq!(view.rows[1].attempt, 1, "the first row of m2 is kept");
    assert!(view.more && view.wants_more(2));

    model.more();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
    assert_eq!(ids(&view), ["m1", "m2", "m3", "m4"]);
    assert_eq!(view.rows[2].attempt, 1, "the first row of m3 is kept");
    let asked = served(&mut server, "served /messages");
    assert_eq!(
        asked,
        [
            "served /messages?limit=100",
            "served /messages?cursor=c1&limit=100",
            "served /messages?cursor=c2&limit=100",
            "served /messages?cursor=c3&limit=100",
        ]
    );
}

/// Scenario: A pending message. The pane is filled from the selected row, with no read of its own; its payload is
/// formatted only then. A search that leaves out the message's target unselects it, and Refresh clears the selection.
#[tokio::test]
async fn the_pane_comes_from_the_row_and_formats_the_payload_only_on_selection() {
    let actors = actors().await;
    let rows = [envelope("m1", "bag/a", 1), envelope("m2", "bag/b", 2)];
    let first = page(&rows, None);
    let rule: Rule = Arc::new(move |target| target.starts_with("/messages").then(|| first.clone()));
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, limits());
    let (mut list, mut pane) = (model.list(), model.pane());
    model.show();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    assert_eq!(model.formatted(), 0, "listing formats no payload");
    assert_eq!(pane.borrow().message, None);
    served(&mut server, "");

    let (m1, m2) = (id("m1"), id("m2"));
    model.select(Some(&m2));
    let shown = pane_until(&mut pane, |p| p.payload.is_some()).await;
    assert_eq!(model.formatted(), 1, "the selection formatted its payload");
    let message = shown.message.as_ref().expect("a message is selected");
    assert!(Arc::ptr_eq(message, &view.rows[1]), "the pane shares the row");
    assert_eq!((message.id.as_ref(), message.to.as_str(), message.attempt, message.failures), ("m2", "bag/b", 2, 1));
    assert_eq!(message.from.as_ref().map(ToString::to_string).as_deref(), Some("bag/sender"));
    assert_eq!(message.correlation_id.as_deref(), Some("order-7"));
    assert_eq!(message.headers.get("trace").map(String::as_str), Some("t-1"));
    assert_eq!(shown.payload.as_ref().unwrap().text, "{\n  \"n\": 2,\n  \"name\": \"zoë\"\n}");
    assert!(served(&mut server, "").is_empty(), "the pane reads nothing: {:?}", served(&mut server, ""));

    model.search("bag/");
    assert_eq!(model.selected(), Some(m2.clone()), "a prefix that holds the target keeps it");
    model.search("bag/a");
    assert_eq!((model.selected(), pane.borrow().message.clone()), (None, None));
    list_until(&mut list, |l| l.paging == Paging::Idle && l.prefix == "bag/a").await;
    model.select(Some(&m1));
    pane_until(&mut pane, |p| p.payload.is_some()).await;
    model.refresh();
    assert_eq!((model.selected(), pane.borrow().message.clone()), (None, None));
    model.select(Some(&id("not-a-row")));
    assert_eq!((model.selected(), pane.borrow().message.clone()), (None, None));
}

/// A formatting that is still running when another message is selected finishes, and its result is dropped: the
/// first formatting is held until after the second selection has shown its payload, then let go.
#[tokio::test]
async fn a_formatting_that_ends_after_a_later_selection_is_dropped() {
    let actors = actors().await;
    let first = page(&[envelope("m1", "bag/a", 1), envelope("m2", "bag/b", 2)], None);
    let rule: Rule = Arc::new(move |target| target.starts_with("/messages").then(|| first.clone()));
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, limits());
    let (mut list, mut pane) = (model.list(), model.pane());
    model.show();
    list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;

    let (arrived, release) = model.hold_next_format();
    model.select(Some(&id("m1")));
    let started = tokio::task::spawn_blocking(move || arrived.recv_timeout(WAIT)).await.unwrap();
    started.expect("the first formatting started and is held");
    model.select(Some(&id("m2")));
    let current = pane_until(&mut pane, |p| p.payload.is_some()).await;
    assert_eq!(current.message.as_ref().map(|m| m.id.clone()), Some(id("m2")));
    assert!(current.payload.as_ref().unwrap().text.contains("\"n\": 2"));

    release.send(()).unwrap();
    for task in model.take_aborted() {
        timeout(WAIT, task).await.expect("the held formatting ends").expect("a started formatting runs to its end");
    }
    assert_eq!(model.formatted(), 2, "the held formatting ran");
    assert!(!pane.has_changed().unwrap(), "its result was dropped");
    assert_eq!(*pane.borrow(), current);
}

/// A selected message that the pages of a new search do not hold, once every page is read, is unselected and the
/// pane emptied, also when its target is under the new prefix.
#[tokio::test]
async fn a_search_whose_pages_no_longer_hold_the_message_empties_the_pane() {
    let actors = actors().await;
    let all = page(&[envelope("m1", "bag/a", 1), envelope("m2", "bag/b", 1)], None);
    let handled = page(&[envelope("m2", "bag/b", 1)], None);
    let rule: Rule = Arc::new(move |target| match target {
        "/messages?limit=100" => Some(all.clone()),
        "/messages?prefix=bag%2F&limit=100" => Some(handled.clone()),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, limits());
    let (mut list, mut pane) = (model.list(), model.pane());
    model.show();
    list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    model.select(Some(&id("m1")));
    pane_until(&mut pane, |p| p.payload.is_some()).await;

    model.search("bag/");
    assert_eq!(model.selected(), Some(id("m1")), "the target is under the prefix");
    let view = list_until(&mut list, |l| l.prefix == "bag/" && l.paging == Paging::Idle).await;
    assert_eq!((ids(&view), view.more), (vec!["m2".to_owned()], false));
    assert_eq!(model.selected(), None);
    let emptied = pane.borrow().clone();
    assert_eq!((emptied.message, emptied.payload), (None, None));
}

/// A session on the server of `server`, on the test's [`limits`].
fn session(server: &Server, dir: &tempfile::TempDir) -> Session {
    let env = config(dir, &[("prod", &server.url)]);
    let session = Session::new(env, tokio::runtime::Handle::current(), limits());
    session.use_context(None);
    session
}

fn root(address: &str) -> ActorPath {
    ActorPath::root(ActorAddress::parse(address).unwrap())
}

async fn actors_until(list: &mut watch::Receiver<ListView>, done: impl FnMut(&ListView) -> bool) -> ListView {
    timeout(WAIT, list.wait_for(done)).await.expect("the Actors list changes in time").unwrap().clone()
}

async fn actor_pane_until(pane: &mut watch::Receiver<PaneView>, done: impl FnMut(&PaneView) -> bool) -> PaneView {
    timeout(WAIT, pane.wait_for(done)).await.expect("the Actors pane changes in time").unwrap().clone()
}

/// Scenario: Show Actor. From a message on the second page, Show Actor selects exactly its actor's root row on the
/// Actors page, under a search for its address that also holds a longer address. Back returns to Messages with its
/// prefix, both pages and the message; Forward goes to the actor again, even after the Actors search changed.
#[tokio::test]
async fn show_actor_selects_the_exact_row_and_back_returns_to_the_message_on_a_later_page() {
    let actors = actors().await;
    for n in 0..120 {
        let counter = Counter { id: format!("{n:03}") };
        actors.create(&counter, &json!({})).await;
        actors.send(&counter, Work::Ok).await;
    }
    actors.create(&Counter { id: "1100".into() }, &json!({})).await;
    let server = serve(&actors, no_rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, |navigation| {
        navigation.select(Section::Messages);
        true
    });
    let (messages, mut list) = (session.messages(), session.messages().list());
    messages.search("counter/");
    list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 100).await;
    messages.more();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 120).await;
    let message = view.rows[110].clone();
    assert_eq!(message.to.as_str(), "counter/110");
    messages.select(Some(&message.id));

    let (mut actor_list, mut actor_pane) = (session.actors().list(), session.actors().pane());
    session.show_actor(&mut navigation, message.to.clone());
    assert_eq!((navigation.current(), navigation.place().actor.as_ref()), (Section::Actors, Some(&message.to)));
    let shown = actor_pane_until(&mut actor_pane, |p| p.address.as_ref() == Some(&message.to)).await;
    assert!(!shown.not_found());
    assert_eq!(session.actors().selected(), Some(root("counter/110")));
    let rows: Vec<_> = actor_list.borrow_and_update().rows.iter().map(|r| r.address.to_string()).collect();
    assert_eq!(rows, ["counter/110", "counter/1100"]);
    assert_eq!(actor_list.borrow().prefix, "counter/110");

    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!(navigation.current(), Section::Messages);
    assert!(!session.actors().jumping());
    let back = list.borrow().clone();
    assert_eq!((back.prefix.as_str(), back.rows.len()), ("counter/", 120));
    assert_eq!(messages.selected(), Some(message.id.clone()));
    assert!(messages.pane().borrow().message.as_ref().is_some_and(|m| Arc::ptr_eq(m, &message)));

    session.actors().search("sync/");
    assert_eq!(session.actors().selected(), None);
    assert!(session.navigate(&mut navigation, Navigation::forward));
    assert_eq!((navigation.current(), navigation.place().actor.as_ref()), (Section::Actors, Some(&message.to)));
    assert_eq!(actor_list.borrow().prefix, "counter/110", "Forward searches for the address again");
    actor_pane_until(&mut actor_pane, |p| p.address.as_ref() == Some(&message.to) && !p.not_found()).await;
    assert_eq!(session.actors().selected(), Some(root("counter/110")));
    actors_until(&mut actor_list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    session.stop();
}

/// Show Actor for a gone actor says "not found" once the page loaded, with no row selected. A failed page selects
/// nothing and shows its error; Retry keeps the target and then selects the actor.
#[tokio::test]
async fn show_actor_says_not_found_for_a_gone_actor_and_a_failed_read_selects_nothing_until_retry() {
    let actors = actors().await;
    actors.create(&Counter { id: "a".into() }, &json!({})).await;
    let fails = Arc::new(AtomicBool::new(true));
    let failing = fails.clone();
    let rule: Rule = Arc::new(move |target| {
        let first = target == "/actors/states?prefix=counter%2Fa&limit=100";
        (first && failing.swap(false, Ordering::SeqCst)).then(|| problem(500))
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let mut navigation = Navigation::default();
    let (mut list, mut pane) = (session.actors().list(), session.actors().pane());

    let ghost = ActorAddress::parse("counter/ghost").unwrap();
    session.show_actor(&mut navigation, ghost.clone());
    let gone = actor_pane_until(&mut pane, |p| p.not_found()).await;
    assert_eq!(gone.address, Some(ghost));
    assert_eq!(session.actors().selected(), None);
    assert!(list.borrow_and_update().rows.is_empty());

    let a = ActorAddress::parse("counter/a").unwrap();
    session.show_actor(&mut navigation, a.clone());
    let failed = actors_until(&mut list, |l| l.prefix == "counter/a" && matches!(l.paging, Paging::Failed(_))).await;
    assert!(matches!(&failed.paging, Paging::Failed(error) if error.contains("500")), "{:?}", failed.paging);
    assert_eq!((session.actors().selected(), pane.borrow().address.clone()), (None, None));
    assert!(session.actors().jumping(), "the target is kept for Retry");
    session.actors().retry();
    actor_pane_until(&mut pane, |p| p.address.as_ref() == Some(&a) && !p.not_found()).await;
    assert_eq!(session.actors().selected(), Some(ActorPath::root(a)));
    assert!(navigation.can_back());
    session.stop();
}

/// A Show Actor still waiting for its page is cancelled by leaving the Actors page or by a new search: its page
/// still lands, and no row is selected.
#[tokio::test]
async fn a_show_actor_is_cancelled_by_leaving_the_page_or_a_new_search() {
    let actors = actors().await;
    actors.create(&Counter { id: "a".into() }, &json!({})).await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let stall = Fake::Wait { arrived: arrived.clone(), release: release.clone() };
    let rule: Rule = Arc::new(move |t| (t == "/actors/states?prefix=counter%2Fa&limit=100").then(|| stall.clone()));
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let mut navigation = Navigation::default();
    let mut list = session.actors().list();
    let a = ActorAddress::parse("counter/a").unwrap();

    session.show_actor(&mut navigation, a.clone());
    timeout(WAIT, arrived.notified()).await.expect("the page of the Show Actor is held");
    session.navigate(&mut navigation, |navigation| {
        navigation.select(Section::Overview);
        true
    });
    assert!(!session.actors().jumping());
    release.notify_waiters();
    actors_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
    assert_eq!((session.actors().selected(), session.actors().pane().borrow().address.clone()), (None, None));

    session.show_actor(&mut navigation, a);
    timeout(WAIT, arrived.notified()).await.expect("the page of the second Show Actor is held");
    session.actors().search("counter/");
    assert!(!session.actors().jumping());
    release.notify_waiters();
    actors_until(&mut list, |l| l.prefix == "counter/" && l.paging == Paging::Idle && l.rows.len() == 1).await;
    assert_eq!((session.actors().selected(), session.actors().pane().borrow().address.clone()), (None, None));
    session.stop();
}

fn select_messages(navigation: &mut Navigation) -> bool {
    let before = navigation.current();
    navigation.select(Section::Messages);
    navigation.current() != before
}

/// Each Messages place keeps its own prefix and selection. Show Actor from message A under one prefix, a new visit to
/// Messages, Show Actor from message B under another; then Back restores B's place, the jump to A's actor, and A's
/// place with its own prefix and message, read again; Forward replays them in order.
#[tokio::test]
async fn each_messages_place_restores_its_own_prefix_and_message() {
    let actors = actors().await;
    for n in 0..20 {
        let counter = Counter { id: format!("{n:03}") };
        actors.create(&counter, &json!({})).await;
        actors.send(&counter, Work::Ok).await;
    }
    let server = serve(&actors, no_rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let messages = session.messages();
    let (mut list, mut pane) = (messages.list(), messages.pane());
    let mut actor_pane = session.actors().pane();
    let mut navigation = Navigation::default();
    let current = |pane: &MessagePane| pane.message.as_ref().map(|m| m.id.clone());

    session.navigate(&mut navigation, select_messages);
    messages.search("counter/00");
    let view = list_until(&mut list, |l| l.prefix == "counter/00" && l.paging == Paging::Idle && !l.more).await;
    let a = view.rows[3].clone();
    messages.select(Some(&a.id));
    session.show_actor(&mut navigation, a.to.clone());
    actor_pane_until(&mut actor_pane, |p| p.address.as_ref() == Some(&a.to)).await;

    assert!(session.navigate(&mut navigation, select_messages), "a new visit to Messages");
    messages.search("counter/01");
    let view = list_until(&mut list, |l| l.prefix == "counter/01" && l.paging == Paging::Idle && !l.more).await;
    let b = view.rows[5].clone();
    messages.select(Some(&b.id));
    session.show_actor(&mut navigation, b.to.clone());
    actor_pane_until(&mut actor_pane, |p| p.address.as_ref() == Some(&b.to)).await;

    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!((list.borrow().prefix.as_str(), messages.selected()), ("counter/01", Some(b.id.clone())));
    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!(navigation.place().actor.as_ref(), Some(&a.to));
    actor_pane_until(&mut actor_pane, |p| p.address.as_ref() == Some(&a.to)).await;
    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!((navigation.current(), list.borrow().prefix.as_str()), (Section::Messages, "counter/00"));
    pane_until(&mut pane, |p| current(p) == Some(a.id.clone())).await;
    assert_eq!(messages.selected(), Some(a.id.clone()));

    assert!(session.navigate(&mut navigation, Navigation::forward));
    actor_pane_until(&mut actor_pane, |p| p.address.as_ref() == Some(&a.to)).await;
    assert!(session.navigate(&mut navigation, Navigation::forward));
    assert_eq!(list.borrow().prefix, "counter/01");
    pane_until(&mut pane, |p| current(p) == Some(b.id.clone())).await;
    session.stop();
}

/// A context switch forgets the history: no place of the context before can be gone back or forward to, and the
/// current place no longer jumps, so no same-named actor of the other server is selected.
#[tokio::test]
async fn a_context_switch_forgets_the_history() {
    let (prod_actors, staging_actors) = (actors().await, actors().await);
    for actors in [&prod_actors, &staging_actors] {
        let counter = Counter { id: "a".into() };
        actors.create(&counter, &json!({})).await;
        actors.send(&counter, Work::Ok).await;
    }
    let prod = serve(&prod_actors, no_rule(), Duration::ZERO).await;
    let staging = serve(&staging_actors, no_rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let env = config(&dir, &[("prod", &prod.url), ("staging", &staging.url)]);
    let session = Session::new(env, tokio::runtime::Handle::current(), limits());
    session.use_context(None);
    let mut navigation = Navigation::default();
    let mut actor_pane = session.actors().pane();
    let a = ActorAddress::parse("counter/a").unwrap();

    session.navigate(&mut navigation, select_messages);
    session.show_actor(&mut navigation, a.clone());
    actor_pane_until(&mut actor_pane, |p| p.address.as_ref() == Some(&a)).await;
    assert!(session.navigate(&mut navigation, Navigation::back));
    assert!(navigation.can_forward());

    session.switch_context(&mut navigation, Some("staging".into()));
    assert!(!navigation.can_back() && !navigation.can_forward());
    assert!(!session.navigate(&mut navigation, Navigation::forward), "the jump of prod is not replayed");
    assert_eq!(navigation.place().messages, None, "nothing of prod is restored");
    assert!(!session.actors().jumping());

    session.show_actor(&mut navigation, a.clone());
    session.switch_context(&mut navigation, Some("prod".into()));
    assert_eq!((navigation.current(), navigation.place().actor.clone()), (Section::Actors, None));
    session.navigate(&mut navigation, |navigation| {
        navigation.select(Section::Overview);
        true
    });
    assert!(session.navigate(&mut navigation, Navigation::back), "back to the Actors place of the switch");
    assert_eq!((navigation.current(), navigation.place().actor.clone()), (Section::Actors, None));
    assert!(!navigation.can_back(), "the places before the switch are gone");
    assert!(!session.actors().jumping());
    assert_eq!(session.actors().selected(), None);
    session.stop();
}
