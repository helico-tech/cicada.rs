//! The Outcomes page's model against an in-process server: the list by status and prefix, the sidebar's failed count,
//! the pane from the row, and the cause chain as places of the history. Real sockets and the real clock, as in the
//! parent module, but no outcome depends on the machine's speed: a search reads at once unless a test holds it, a
//! read's deadline is far past every wait, answers that must come in an order are held, and every wait has a bound.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::api::MessageState;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use tokio::time::timeout;

use super::messages::{envelope, id};
use super::{Fake, Rule, Server, Syncer, Work, actors, config, no_rule, problem, serve};
use crate::model::{Limits, Navigation, Section, Session};
use crate::outcomes::{
    FailedCount, OutcomePane, Outcomes, OutcomesView, Read, StatusFilter, Subject, first_line, retry_text,
};
use crate::pager::Paging;

/// The longest a test waits for one change.
const WAIT: Duration = Duration::from_secs(10);

/// Searches read at once, and a read times out only well after a wait would have failed.
fn limits() -> Limits {
    Limits { debounce: Duration::ZERO, deadline: 6 * WAIT, ..Limits::DEFAULT }
}

fn model(server: &Server, limits: Limits) -> Outcomes {
    let outcomes = Outcomes::new(tokio::runtime::Handle::current(), limits);
    outcomes.use_client(Some(server.client.clone()));
    outcomes
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

/// A served outcome whose message `caused_by` sent.
pub(super) fn outcome(id: &str, actor: &str, status: &str, caused_by: Option<&str>) -> Value {
    let error = (status == "failed").then(|| format!("broken {id}\n  at handler"));
    let mut message = envelope(id, actor, 2);
    message["causedBy"] = json!(caused_by);
    message["lastError"] = json!("timeout");
    message["retry"] = json!({"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT5S"}});
    json!({"id": id, "actor": actor, "status": status, "error": error, "finishedAt": "2026-11-27T10:05:00.000Z",
           "message": message})
}

/// A served `GET /messages/{id}`.
pub(super) fn status(id: &str, state: &str) -> Value {
    let finished = (state != "pending").then_some("2026-11-27T10:06:00.000Z");
    json!({"id": id, "status": state, "finishedAt": finished, "message": envelope(id, "bag/cause", 1)})
}

fn page(items: &[Value], next: Option<&str>) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"items": items, "next": next}))
}

fn not_found() -> Fake {
    let body = json!({"type": "urn:cicada:problem:no-such-message", "title": "Not Found", "status": 404});
    Fake::Answer(StatusCode::NOT_FOUND, body)
}

fn routes(routes: Vec<(&'static str, Fake)>) -> Rule {
    Arc::new(move |target| routes.iter().find(|(path, _)| *path == target).map(|(_, fake)| fake.clone()))
}

fn ids(view: &OutcomesView) -> Vec<String> {
    view.rows.iter().map(|row| row.id.to_string()).collect()
}

fn served(server: &mut Server, prefix: &str) -> Vec<String> {
    std::iter::from_fn(|| server.events.try_recv().ok()).filter(|e| e.starts_with(prefix)).collect()
}

fn shown(pane: &OutcomePane) -> Option<String> {
    pane.subject.as_ref().map(|subject| subject.id().to_string())
}

fn select_outcomes(navigation: &mut Navigation) -> bool {
    let before = navigation.current();
    navigation.select(Section::Outcomes);
    navigation.current() != before
}

/// Scenario: Failed outcomes. Two messages failed and one is done: the table lists the two failed ones and the sidebar
/// shows 2. The switch shows Done, and All with every status.
#[tokio::test]
async fn failed_outcomes_are_listed_by_default_and_the_sidebar_counts_them() {
    let actors = actors().await;
    for key in ["a", "b", "c"] {
        actors.create(&Syncer { id: key.into() }, &json!({})).await;
    }
    let a = actors.send(&Syncer { id: "a".into() }, Work::Fail).await;
    let b = actors.send(&Syncer { id: "b".into() }, Work::Fail).await;
    let c = actors.send(&Syncer { id: "c".into() }, Work::Ok).await;
    actors.run_until_idle().await;
    let server = serve(&actors, no_rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let mut navigation = Navigation::default();
    let (outcomes, mut list, mut count) = (session.outcomes(), session.outcomes().list(), session.outcomes().count());
    session.navigate(&mut navigation, select_outcomes);
    outcomes.show();

    let view = until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
    assert_eq!(view.status, StatusFilter::Failed);
    let mut listed = ids(&view);
    listed.sort();
    let mut failed = vec![a.id.to_string(), b.id.to_string()];
    failed.sort();
    assert_eq!(listed, failed);
    let row = &view.rows[0];
    assert_eq!((row.status, row.message.message_type.as_str(), row.message.attempt), (MessageState::Failed, "Fail", 1));
    assert_eq!(first_line(row.error.as_deref().unwrap()), "broken");
    let count = until(&mut count, |c| c.value.is_some()).await;
    assert_eq!(
        (count.text(), count.tooltip()),
        ("2".to_owned(), "failed, kept by retention, as of the last read".into())
    );

    outcomes.set_status(StatusFilter::Done);
    let view = until(&mut list, |l| l.status == StatusFilter::Done && l.paging == Paging::Idle && !l.more).await;
    assert_eq!(ids(&view), [c.id.to_string()]);
    outcomes.set_status(StatusFilter::All);
    let view = until(&mut list, |l| l.status == StatusFilter::All && l.paging == Paging::Idle && !l.more).await;
    assert_eq!(view.rows.len(), 3);
    assert!(view.rows.iter().any(|row| row.status == MessageState::Done));
    session.stop();
}

/// The list by status and prefix, by the unchanged cursor. A message on two pages is shown once, the first row kept;
/// a page of only duplicates leaves the rows as they were, yet the next page is asked for at once.
#[tokio::test]
async fn outcomes_page_by_the_cursor_and_each_message_shows_once() {
    let actors = actors().await;
    let rule = routes(vec![
        (
            "/outcomes?prefix=bag%2F&limit=100&status=failed",
            page(&[outcome("m1", "bag/a", "failed", None), outcome("m2", "bag/b", "failed", None)], Some("c1")),
        ),
        (
            "/outcomes?prefix=bag%2F&cursor=c1&limit=100&status=failed",
            page(&[outcome("m2", "bag/b", "cancelled", None), outcome("m3", "bag/c", "failed", None)], Some("c2")),
        ),
        (
            "/outcomes?prefix=bag%2F&cursor=c2&limit=100&status=failed",
            page(&[outcome("m3", "bag/c", "done", None)], Some("c3")),
        ),
        (
            "/outcomes?prefix=bag%2F&cursor=c3&limit=100&status=failed",
            page(&[outcome("m4", "bag/d", "failed", None)], None),
        ),
    ]);
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, limits());
    let mut list = model.list();
    model.search("bag/");
    let first = until(&mut list, |l| l.paging == Paging::Idle && !l.rows.is_empty()).await;
    assert_eq!((ids(&first), first.more, first.prefix.as_str()), (vec!["m1".to_owned(), "m2".into()], true, "bag/"));
    assert!(first.wants_more(0));
    model.more();
    model.more();
    let view = until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 3).await;
    assert_eq!(view.rows[1].status, MessageState::Failed, "the first row of m2 is kept");
    assert!(Arc::ptr_eq(&first.rows[0], &view.rows[0]), "a later view shares the rows read before");
    model.more();
    let view = until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
    assert_eq!(ids(&view), ["m1", "m2", "m3", "m4"]);
    assert_eq!(view.rows[2].status, MessageState::Failed, "the first row of m3 is kept");
    let asked = served(&mut server, "served /outcomes");
    assert_eq!(
        asked,
        [
            "served /outcomes?prefix=bag%2F&limit=100&status=failed",
            "served /outcomes?prefix=bag%2F&cursor=c1&limit=100&status=failed",
            "served /outcomes?prefix=bag%2F&cursor=c2&limit=100&status=failed",
            "served /outcomes?prefix=bag%2F&cursor=c3&limit=100&status=failed",
        ]
    );
}

/// The switch resets together the cursor, the seen ids, a pending search, the selection and the pane's work: the
/// Done page is read from its start at once, with the typed prefix, and holds a message seen under Failed; the search
/// still waiting is dropped; the pane is emptied and its held Caused read is dropped. All asks every status.
#[tokio::test]
async fn the_switch_resets_the_cursor_seen_ids_search_selection_and_pane_work() {
    let actors = actors().await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held = Fake::Later {
        arrived: arrived.clone(),
        release: release.clone(),
        status: StatusCode::OK,
        body: json!({"items": [status("x1", "pending")], "next": null}),
    };
    let rule = routes(vec![
        ("/outcomes?limit=100&status=failed", page(&[outcome("m1", "bag/a", "failed", None)], Some("c1"))),
        ("/outcomes?prefix=bag%2F&limit=100&status=done", page(&[outcome("m1", "bag/a", "done", None)], Some("d1"))),
        ("/outcomes?prefix=bag%2F&limit=100", page(&[outcome("m1", "bag/a", "purged", None)], None)),
        ("/messages/m1/caused?limit=50", held),
    ]);
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    // The search waits far longer than the test, so it is still pending at the switch.
    let model = model(&server, Limits { debounce: 6 * WAIT, ..limits() });
    let (mut list, mut pane) = (model.list(), model.pane());
    model.show();
    until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
    model.select(Some(&id("m1")));
    timeout(WAIT, arrived.notified()).await.expect("the Caused read of m1 is held");
    model.search("bag/");
    model.take_aborted();

    model.set_status(StatusFilter::Done);
    assert_eq!((model.selected(), pane.borrow().subject.clone()), (None, None));
    let view = until(&mut list, |l| l.status == StatusFilter::Done && l.paging == Paging::Idle).await;
    assert_eq!((ids(&view), view.prefix.as_str(), view.more), (vec!["m1".to_owned()], "bag/", true));
    let aborted = model.take_aborted();
    assert!(aborted.len() >= 2, "the pending search and the Caused read were dropped: {}", aborted.len());
    release.notify_one();
    for task in aborted {
        let ended = timeout(WAIT, task).await.expect("a dropped task ends");
        assert!(ended.as_ref().is_err_and(|error| error.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    assert!(pane.borrow_and_update().subject.is_none(), "the held Caused read publishes nothing");

    model.set_status(StatusFilter::All);
    let view = until(&mut list, |l| l.status == StatusFilter::All && l.paging == Paging::Idle).await;
    assert_eq!((view.rows[0].status, view.more), (MessageState::Purged, false));
    let asked = served(&mut server, "served /outcomes");
    assert_eq!(
        asked,
        [
            "served /outcomes?limit=100&status=failed",
            "served /outcomes?prefix=bag%2F&limit=100&status=done",
            "served /outcomes?prefix=bag%2F&limit=100",
        ],
        "the Done page starts at its first cursor, and the search never read Failed under the prefix"
    );
}

/// The sidebar count: "…" until the first read answers, the number, the last number kept with the error after a
/// failed read, "—" from an older server, and "…" again after a context switch, whose count read before is dropped.
/// A first read that fails shows "—" with its error.
#[tokio::test]
async fn the_failed_count_waits_keeps_its_value_on_errors_and_resets_on_a_context_switch() {
    let (prod_actors, staging_actors) = (actors().await, actors().await);
    let mode = Arc::new(AtomicUsize::new(0));
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let (rule_mode, stall) = (mode.clone(), (arrived.clone(), release.clone()));
    let prod_rule: Rule = Arc::new(move |target| {
        (target == "/stats").then(|| match rule_mode.load(Ordering::SeqCst) {
            0 => Fake::Later {
                arrived: stall.0.clone(),
                release: stall.1.clone(),
                status: StatusCode::OK,
                body: json!({"actors": 0, "pending": 0, "oldestDue": null, "failed": 7}),
            },
            1 => problem(500),
            _ => Fake::Answer(StatusCode::OK, json!({"actors": 0, "pending": 0, "oldestDue": null})),
        })
    });
    let prod = serve(&prod_actors, prod_rule, Duration::ZERO).await;
    let staging = serve(&staging_actors, Arc::new(|t| (t == "/stats").then(|| problem(503))), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let env = config(&dir, &[("prod", &prod.url), ("staging", &staging.url)]);
    let session = Session::new(env, tokio::runtime::Handle::current(), limits());
    let mut count = session.outcomes().count();

    session.use_context(None);
    timeout(WAIT, arrived.notified()).await.expect("the Overview's /stats is held");
    assert_eq!(count.borrow_and_update().text(), "…");
    release.notify_one();
    let ready = until(&mut count, |c| c.value.is_some()).await;
    assert_eq!((ready.text(), ready.error), ("7".to_owned(), None));

    mode.store(1, Ordering::SeqCst);
    session.outcomes().refresh();
    let kept = until(&mut count, |c| c.error.is_some()).await;
    assert_eq!(kept.text(), "7");
    assert!(kept.tooltip().starts_with("failed, kept by retention, as of the last read; the last read failed: "));
    assert!(kept.tooltip().contains("500"), "{}", kept.tooltip());

    mode.store(2, Ordering::SeqCst);
    session.refresh();
    let older = until(&mut count, |c| c.value == Some(None)).await;
    assert_eq!((older.text(), older.error), ("—".to_owned(), None));

    // A count read held at the switch is dropped, and the new context's first read fails.
    mode.store(0, Ordering::SeqCst);
    session.outcomes().refresh();
    timeout(WAIT, arrived.notified()).await.expect("the Outcomes refresh's /stats is held");
    let mut navigation = Navigation::default();
    session.switch_context(&mut navigation, Some("staging".into()));
    assert_eq!(*count.borrow_and_update(), FailedCount::default(), "the switch clears the count");
    let failed = until(&mut count, |c| c.error.is_some()).await;
    assert_eq!((failed.text(), failed.value), ("—".to_owned(), None));
    assert!(failed.tooltip().contains("503"), "{}", failed.tooltip());
    release.notify_one();
    for task in session.outcomes().take_aborted() {
        let ended = timeout(WAIT, task).await.expect("the old count read ends");
        assert!(ended.as_ref().is_err_and(|error| error.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    assert!(!count.has_changed().unwrap(), "the old count read publishes nothing");
    session.stop();
}

/// The pane comes from the selected row with no read of the message; its payload is formatted only then. The row's
/// retry policy, attempts and last retried error are shown, and Caused is read.
#[tokio::test]
async fn the_pane_comes_from_the_row_and_formats_the_payload_only_on_selection() {
    let actors = actors().await;
    let rule = routes(vec![
        ("/outcomes?limit=100&status=failed", page(&[outcome("m1", "bag/a", "failed", Some("p1"))], None)),
        ("/messages/m1/caused?limit=50", page(&[status("x1", "pending")], None)),
    ]);
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, limits());
    let (mut list, mut pane) = (model.list(), model.pane());
    model.show();
    let view = until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
    assert_eq!(model.formatted(), 0, "listing formats no payload");
    served(&mut server, "");

    model.select(Some(&id("m1")));
    let shown = until(&mut pane, |p| p.payload.is_some() && p.caused.paging == Paging::Idle).await;
    assert_eq!(model.formatted(), 1);
    let Some(Subject::Outcome(outcome)) = &shown.subject else { panic!("{:?}", shown.subject) };
    assert!(Arc::ptr_eq(outcome, &view.rows[0]), "the pane shares the row");
    let details = shown.subject.as_ref().unwrap().details().unwrap();
    assert_eq!((details.status, details.error), (MessageState::Failed, Some("broken m1\n  at handler")));
    assert_eq!((details.message.attempt, details.message.failures), (2, 1));
    assert_eq!(details.message.last_error.as_deref(), Some("timeout"));
    assert_eq!(details.message.caused_by, Some(id("p1")));
    assert_eq!(retry_text(details.message.retry.as_ref()), "Up to 3 attempts, 5s apart");
    assert_eq!(retry_text(None), "No retry policy");
    assert_eq!(shown.payload.as_ref().unwrap().text, "{\n  \"n\": 2,\n  \"name\": \"zoë\"\n}");
    assert_eq!(shown.caused.rows.iter().map(|r| r.id.to_string()).collect::<Vec<_>>(), ["x1"]);
    assert_eq!(served(&mut server, "served /messages"), ["served /messages/m1/caused?limit=50"]);

    model.refresh();
    assert_eq!((model.selected(), pane.borrow().subject.clone()), (None, None));
}

/// Scenario: Following the cause; and Show Actor. Caused by is followed to a pending message and back to the failed
/// outcome; Forward reads it again. A finished cause shows its outcome, one past retention "not found". Show Actor
/// jumps to the outcome's actor and Back restores the outcome.
#[tokio::test]
async fn caused_by_is_followed_to_pending_finished_and_gone_messages_as_places() {
    let actors = actors().await;
    let rule = routes(vec![
        (
            "/outcomes?limit=100&status=failed",
            page(&[outcome("m1", "bag/a", "failed", Some("p1")), outcome("m2", "bag/b", "failed", Some("gone"))], None),
        ),
        ("/messages/p1", Fake::Answer(StatusCode::OK, status("p1", "pending"))),
        ("/messages/p1/caused?limit=50", page(&[], None)),
        ("/messages/f1", Fake::Answer(StatusCode::OK, status("f1", "done"))),
        ("/messages/f1/caused?limit=50", page(&[], None)),
        ("/messages/gone", not_found()),
        ("/messages/gone/caused?limit=50", not_found()),
    ]);
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let outcomes = session.outcomes();
    let (mut list, mut pane) = (outcomes.list(), outcomes.pane());
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, select_outcomes);
    outcomes.show();
    let view = until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    outcomes.select(Some(&id("m1")));

    session.follow(&mut navigation, id("p1"));
    assert_eq!(pane.borrow().subject, Some(Subject::Linked(id("p1"), Read::Loading)));
    assert_eq!(outcomes.selected(), None);
    let linked = until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;
    assert!(!linked.caused.none(), "Caused shows loading, not None, until its first page answers");
    let details = linked.subject.as_ref().unwrap().details().unwrap();
    assert_eq!((details.status, details.finished_at), (MessageState::Pending, None));
    until(&mut pane, |p| p.payload.is_some() && p.caused.none()).await;

    assert!(session.navigate(&mut navigation, Navigation::back));
    let back = pane.borrow().clone();
    assert!(matches!(&back.subject, Some(Subject::Outcome(o)) if Arc::ptr_eq(o, &view.rows[0])), "{back:?}");
    assert_eq!(outcomes.selected(), Some(id("m1")), "its row is loaded, so it is selected");
    served(&mut server, "");
    assert!(session.navigate(&mut navigation, Navigation::forward));
    until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;
    assert_eq!(served(&mut server, "served /messages/p1").len(), 1, "Forward reads the message fresh");

    session.follow(&mut navigation, id("f1"));
    let finished = until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;
    assert_eq!(finished.subject.as_ref().unwrap().details().unwrap().status, MessageState::Done);

    session.follow(&mut navigation, id("gone"));
    until(&mut pane, |p| p.subject == Some(Subject::Linked(id("gone"), Read::NotFound))).await;
    assert!(navigation.can_back() && !navigation.can_forward());

    // Back to m1 three steps back; then Show Actor from it, and Back to it again.
    for _ in 0..3 {
        assert!(session.navigate(&mut navigation, Navigation::back));
    }
    assert_eq!(shown(&pane.borrow()), Some("m1".into()));
    let to = view.rows[0].message.to.clone();
    session.show_actor(&mut navigation, to.clone());
    assert_eq!((navigation.current(), navigation.place().actor.as_ref()), (Section::Actors, Some(&to)));
    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!((navigation.current(), shown(&pane.borrow())), (Section::Outcomes, Some("m1".into())));
    assert_eq!(outcomes.selected(), Some(id("m1")));
    session.stop();
}

/// Caused: its first 50, "more…" by the cursor with one request at a time, Retry on the same cursor, pages with no rows
/// but a next cursor followed at once (two in a row), "None" only when the list ends with no rows, and "not found" for
/// a gone parent.
#[tokio::test]
async fn caused_pages_one_at_a_time_retry_the_same_cursor_and_follow_empty_pages() {
    let actors = actors().await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let tries = Arc::new(AtomicUsize::new(0));
    let (rule_tries, stall) = (tries.clone(), (arrived.clone(), release.clone()));
    let fixed = routes(vec![
        (
            "/outcomes?limit=100&status=failed",
            page(
                &[
                    outcome("m1", "bag/a", "failed", None),
                    outcome("m2", "bag/b", "failed", None),
                    outcome("m3", "bag/c", "failed", None),
                    outcome("m4", "bag/d", "failed", None),
                ],
                None,
            ),
        ),
        ("/messages/m1/caused?limit=50", page(&[status("x1", "pending"), status("x2", "done")], Some("k1"))),
        ("/messages/m1/caused?cursor=k2&limit=50", page(&[], Some("k3"))),
        ("/messages/m1/caused?cursor=k3&limit=50", page(&[status("x3", "failed")], None)),
        ("/messages/m2/caused?limit=50", page(&[], None)),
        ("/messages/m3/caused?limit=50", not_found()),
        ("/messages/m4/caused?limit=50", page(&[], Some("e1"))),
        ("/messages/m4/caused?cursor=e1&limit=50", page(&[], Some("e2"))),
        ("/messages/m4/caused?cursor=e2&limit=50", page(&[], None)),
    ]);
    let rule: Rule = Arc::new(move |target| {
        if target != "/messages/m1/caused?cursor=k1&limit=50" {
            return fixed(target);
        }
        Some(match rule_tries.fetch_add(1, Ordering::SeqCst) {
            0 => Fake::Later {
                arrived: stall.0.clone(),
                release: stall.1.clone(),
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: json!({"type": "about:blank", "title": "Internal Server Error", "status": 500}),
            },
            _ => page(&[], Some("k2")),
        })
    });
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, limits());
    let (mut list, mut pane) = (model.list(), model.pane());
    model.show();
    until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 4).await;

    model.select(Some(&id("m1")));
    let first = until(&mut pane, |p| p.caused.paging == Paging::Idle).await;
    assert_eq!((first.caused.rows.len(), first.caused.more, first.caused.none()), (2, true, false));
    model.more_caused();
    timeout(WAIT, arrived.notified()).await.expect("the second page is held");
    model.more_caused();
    model.retry_caused();
    release.notify_one();
    let failed = until(&mut pane, |p| matches!(p.caused.paging, Paging::Failed(_))).await;
    assert_eq!(failed.caused.rows.len(), 2);
    model.more_caused();
    model.retry_caused();
    let done = until(&mut pane, |p| p.caused.paging == Paging::Idle && !p.caused.more).await;
    assert_eq!(done.caused.rows.iter().map(|r| r.id.to_string()).collect::<Vec<_>>(), ["x1", "x2", "x3"]);
    assert_eq!(
        served(&mut server, "served /messages/m1"),
        [
            "served /messages/m1/caused?limit=50",
            "served /messages/m1/caused?cursor=k1&limit=50",
            "served /messages/m1/caused?cursor=k1&limit=50",
            "served /messages/m1/caused?cursor=k2&limit=50",
            "served /messages/m1/caused?cursor=k3&limit=50",
        ],
        "one request at a time, Retry on the same cursor, the empty page followed"
    );

    model.select(Some(&id("m2")));
    let none = until(&mut pane, |p| shown(p) == Some("m2".into()) && p.caused.paging == Paging::Idle).await;
    assert!(none.caused.none());
    model.select(Some(&id("m3")));
    let gone = until(&mut pane, |p| shown(p) == Some("m3".into()) && p.caused.paging == Paging::Idle).await;
    assert!(gone.caused.gone && !gone.caused.none());
    model.select(Some(&id("m4")));
    let empty = until(&mut pane, |p| shown(p) == Some("m4".into()) && p.caused.paging == Paging::Idle).await;
    assert!(empty.caused.none(), "{:?}", empty.caused);
    assert_eq!(served(&mut server, "served /messages/m4").len(), 3, "two empty pages in a row are followed");
}

/// A linked read that fails shows its error with Retry, which reads the same id again; apart from "not found". Also
/// from a place reached by back and forward.
#[tokio::test]
async fn a_failed_linked_read_shows_retry_also_from_back_and_forward() {
    let actors = actors().await;
    let reads = Arc::new(AtomicUsize::new(0));
    let rule_reads = reads.clone();
    let fixed = routes(vec![(
        "/outcomes?limit=100&status=failed",
        page(&[outcome("m1", "bag/a", "failed", Some("p1"))], None),
    )]);
    let rule: Rule = Arc::new(move |target| {
        if target != "/messages/p1" {
            return fixed(target);
        }
        let n = rule_reads.fetch_add(1, Ordering::SeqCst);
        Some(if n.is_multiple_of(2) { problem(500) } else { Fake::Answer(StatusCode::OK, status("p1", "failed")) })
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let outcomes = session.outcomes();
    let (mut list, mut pane) = (outcomes.list(), outcomes.pane());
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, select_outcomes);
    outcomes.show();
    until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
    outcomes.select(Some(&id("m1")));

    session.follow(&mut navigation, id("p1"));
    let failed = until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Failed(_))))).await;
    let Some(Subject::Linked(_, Read::Failed(error))) = &failed.subject else { unreachable!() };
    assert!(error.contains("500"), "{error}");
    assert!(failed.subject.as_ref().unwrap().details().is_none() && failed.caused.rows.is_empty());
    outcomes.retry_link();
    until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;

    assert!(session.navigate(&mut navigation, Navigation::back));
    assert!(session.navigate(&mut navigation, Navigation::forward));
    until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Failed(_))))).await;
    outcomes.retry_link();
    let found = until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;
    assert_eq!(found.subject.as_ref().unwrap().id(), &id("p1"));
    assert_eq!(reads.load(Ordering::SeqCst), 4);
    session.stop();
}

/// Scenario: Back to an outcome no longer listed. After Caused by and a switch to Done, Back shows the failed outcome
/// as it was read, with no row selected; the switch left the link's place as it was, so Forward reads the linked
/// message fresh again.
#[tokio::test]
async fn back_and_forward_across_a_status_switch_restore_the_outcome_and_the_link() {
    let actors = actors().await;
    let rule = routes(vec![
        ("/outcomes?limit=100&status=failed", page(&[outcome("m1", "bag/a", "failed", Some("p1"))], None)),
        ("/outcomes?limit=100&status=done", page(&[outcome("d1", "bag/d", "done", None)], None)),
        ("/messages/p1", Fake::Answer(StatusCode::OK, status("p1", "done"))),
        ("/messages/p1/caused?limit=50", page(&[], None)),
    ]);
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let outcomes = session.outcomes();
    let (mut list, mut pane) = (outcomes.list(), outcomes.pane());
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, select_outcomes);
    outcomes.show();
    let view = until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
    outcomes.select(Some(&id("m1")));
    session.follow(&mut navigation, id("p1"));
    until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;
    outcomes.set_status(StatusFilter::Done);
    until(&mut list, |l| l.status == StatusFilter::Done && l.paging == Paging::Idle && l.rows.len() == 1).await;

    assert!(session.navigate(&mut navigation, Navigation::back));
    let back = until(&mut pane, |p| p.payload.is_some()).await;
    assert!(matches!(&back.subject, Some(Subject::Outcome(o)) if Arc::ptr_eq(o, &view.rows[0])), "{back:?}");
    assert_eq!(back.subject.as_ref().unwrap().details().unwrap().status, MessageState::Failed);
    assert_eq!((outcomes.selected(), list.borrow().status), (None, StatusFilter::Done));

    served(&mut server, "");
    assert!(session.navigate(&mut navigation, Navigation::forward));
    assert_eq!(pane.borrow().subject, Some(Subject::Linked(id("p1"), Read::Loading)));
    let forward = until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;
    assert_eq!(forward.subject.as_ref().unwrap().id(), &id("p1"));
    assert_eq!(served(&mut server, "served /messages/p1").first().map(String::as_str), Some("served /messages/p1"));
    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!(shown(&pane.borrow()), Some("m1".into()));
    session.stop();
}

/// Back to an outcome whose row is not loaded shows it as it was read; once paging brings its row, the row is
/// selected and the pane keeps the outcome. When the user selected another row first, that selection stays.
#[tokio::test]
async fn a_restored_outcome_is_selected_once_a_page_brings_its_row() {
    let actors = actors().await;
    let rule = routes(vec![
        ("/outcomes?limit=100&status=failed", page(&[outcome("m1", "bag/a", "failed", None)], Some("c1"))),
        ("/outcomes?cursor=c1&limit=100&status=failed", page(&[outcome("m2", "bag/b", "failed", Some("p1"))], None)),
        ("/messages/p1", Fake::Answer(StatusCode::OK, status("p1", "done"))),
    ]);
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let outcomes = session.outcomes();
    let (mut list, mut pane) = (outcomes.list(), outcomes.pane());
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, select_outcomes);
    outcomes.show();
    until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
    outcomes.more();
    let view = until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
    let m2_id = view.rows[1].id.clone();
    for user_selects_first in [false, true] {
        // The row as the list holds it now: each reload reads it as a new row.
        let m2 = list.borrow().rows.iter().find(|row| row.id == m2_id).cloned().expect("m2 is listed");
        outcomes.select(Some(&m2.id));
        session.follow(&mut navigation, id("p1"));
        until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;
        // Refresh on the link's place reads the list from its first page again, which does not hold m2.
        outcomes.refresh();
        let first = until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
        assert!(first.index(&m2.id).is_none());

        assert!(session.navigate(&mut navigation, Navigation::back));
        let back = pane.borrow().clone();
        assert!(matches!(&back.subject, Some(Subject::Outcome(o)) if Arc::ptr_eq(o, &m2)), "{back:?}");
        assert_eq!(outcomes.selected(), None, "its row is not loaded");
        if user_selects_first {
            outcomes.select(Some(&id("m1")));
        }
        outcomes.more();
        until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
        if user_selects_first {
            assert_eq!((outcomes.selected(), shown(&pane.borrow())), (Some(id("m1")), Some("m1".into())));
        } else {
            assert_eq!(outcomes.selected(), Some(m2.id.clone()), "the row is selected once a page brings it");
            let kept = pane.borrow().clone();
            assert!(matches!(&kept.subject, Some(Subject::Outcome(o)) if Arc::ptr_eq(o, &m2)), "{kept:?}");
        }
    }
    session.stop();
}

/// The failed count shows as soon as the Overview's `/stats` answers, while another route is still out; and a
/// refresh that a later one replaced publishes no count, even when its `/stats` answers last.
#[tokio::test]
async fn the_failed_count_shows_once_stats_answers_and_never_from_a_replaced_refresh() {
    let actors = actors().await;
    let (nodes_arrived, nodes_release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let (stats_arrived, stats_release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let stats_calls = Arc::new(AtomicUsize::new(0));
    let (calls, nodes, stats) = (
        stats_calls.clone(),
        (nodes_arrived.clone(), nodes_release.clone()),
        (stats_arrived.clone(), stats_release.clone()),
    );
    let counted = |failed: u64| json!({"actors": 0, "pending": 0, "oldestDue": null, "failed": failed});
    let rule: Rule = Arc::new(move |target| match target {
        "/nodes" if calls.load(Ordering::SeqCst) <= 1 => Some(Fake::Later {
            arrived: nodes.0.clone(),
            release: nodes.1.clone(),
            status: StatusCode::OK,
            body: json!({"items": []}),
        }),
        "/stats" => Some(match calls.fetch_add(1, Ordering::SeqCst) {
            0 => Fake::Answer(StatusCode::OK, counted(3)),
            1 => Fake::Later {
                arrived: stats.0.clone(),
                release: stats.1.clone(),
                status: StatusCode::OK,
                body: counted(9),
            },
            _ => Fake::Answer(StatusCode::OK, counted(4)),
        }),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let (mut count, mut view) = (session.outcomes().count(), session.view());

    timeout(WAIT, nodes_arrived.notified()).await.expect("the first refresh's /nodes is held");
    let first = until(&mut count, |c| c.value.is_some()).await;
    assert_eq!(first.text(), "3");
    assert!(view.borrow_and_update().overview.is_none(), "the Overview still waits for /nodes");
    nodes_release.notify_one();
    until(&mut view, |v| v.overview.is_some()).await;

    // The second refresh's /stats is held; a third replaces it and answers; then the second answers last.
    session.refresh();
    timeout(WAIT, stats_arrived.notified()).await.expect("the second refresh's /stats is held");
    session.refresh();
    let third = until(&mut count, |c| c.value == Some(Some(4))).await;
    assert_eq!(third.text(), "4");
    stats_release.notify_one();
    for task in session.take_superseded() {
        let ended = timeout(WAIT, task).await.expect("the replaced refresh ends");
        assert!(ended.as_ref().is_err_and(|error| error.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    until(&mut view, |v| !v.refreshing).await;
    assert_eq!(count.borrow_and_update().text(), "4", "the replaced refresh publishes no count");
    session.stop();
}

/// A context switch forgets the history and cancels the pane's reads: a linked read held at the switch publishes
/// nothing, and no place of the context before can be reached.
#[tokio::test]
async fn a_context_switch_clears_the_history_and_cancels_pending_reads() {
    let (prod_actors, staging_actors) = (actors().await, actors().await);
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held = Fake::Later {
        arrived: arrived.clone(),
        release: release.clone(),
        status: StatusCode::OK,
        body: status("p1", "pending"),
    };
    let rule = routes(vec![
        ("/outcomes?limit=100&status=failed", page(&[outcome("m1", "bag/a", "failed", Some("p1"))], None)),
        ("/messages/p1", held),
    ]);
    let prod = serve(&prod_actors, rule, Duration::ZERO).await;
    let staging = serve(&staging_actors, no_rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let env = config(&dir, &[("prod", &prod.url), ("staging", &staging.url)]);
    let session = Session::new(env, tokio::runtime::Handle::current(), limits());
    session.use_context(None);
    let outcomes = session.outcomes();
    let (mut list, mut pane) = (outcomes.list(), outcomes.pane());
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, select_outcomes);
    outcomes.show();
    until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 1).await;
    outcomes.select(Some(&id("m1")));
    session.follow(&mut navigation, id("p1"));
    timeout(WAIT, arrived.notified()).await.expect("the linked read is held");

    session.switch_context(&mut navigation, Some("staging".into()));
    assert!(!navigation.can_back() && !navigation.can_forward());
    assert_eq!(navigation.place().outcomes, None, "nothing of prod is restored");
    assert_eq!(pane.borrow_and_update().subject, None);
    release.notify_one();
    for task in outcomes.take_aborted() {
        let ended = timeout(WAIT, task).await.expect("the held read ends");
        assert!(ended.as_ref().is_err_and(|error| error.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    assert!(!pane.has_changed().unwrap(), "the held read publishes nothing");
    outcomes.show();
    let view = until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
    assert!(view.rows.is_empty(), "staging has no outcomes");
    session.stop();
}

/// A chain read that answers after the pane moved on is dropped: a Caused page of the outcome before, and a linked
/// read left by Back.
#[tokio::test]
async fn a_stale_chain_read_is_dropped() {
    let actors = actors().await;
    let (caused_arrived, caused_release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let (link_arrived, link_release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held_caused = Fake::Later {
        arrived: caused_arrived.clone(),
        release: caused_release.clone(),
        status: StatusCode::OK,
        body: json!({"items": [status("x1", "pending")], "next": null}),
    };
    let held_link = Fake::Later {
        arrived: link_arrived.clone(),
        release: link_release.clone(),
        status: StatusCode::OK,
        body: status("p1", "pending"),
    };
    let rule = routes(vec![
        (
            "/outcomes?limit=100&status=failed",
            page(&[outcome("m1", "bag/a", "failed", Some("p1")), outcome("m2", "bag/b", "failed", None)], None),
        ),
        ("/messages/m1/caused?limit=50", held_caused),
        ("/messages/m2/caused?limit=50", page(&[], None)),
        ("/messages/p1", held_link),
    ]);
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let outcomes = session.outcomes();
    let (mut list, mut pane) = (outcomes.list(), outcomes.pane());
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, select_outcomes);
    outcomes.show();
    until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;

    outcomes.select(Some(&id("m1")));
    timeout(WAIT, caused_arrived.notified()).await.expect("the Caused read of m1 is held");
    outcomes.select(Some(&id("m2")));
    let m2 = until(&mut pane, |p| shown(p) == Some("m2".into()) && p.payload.is_some() && p.caused.none()).await;
    caused_release.notify_one();
    for task in outcomes.take_aborted() {
        let _ = timeout(WAIT, task).await.expect("the held Caused read ends");
    }
    assert_eq!(*pane.borrow_and_update(), m2, "the Caused page of m1 is dropped");

    session.follow(&mut navigation, id("p1"));
    timeout(WAIT, link_arrived.notified()).await.expect("the linked read is held");
    assert!(session.navigate(&mut navigation, Navigation::back));
    let back = pane.borrow_and_update().clone();
    assert_eq!(shown(&back), Some("m2".into()));
    link_release.notify_one();
    for task in outcomes.take_aborted() {
        let _ = timeout(WAIT, task).await.expect("the held linked read ends");
    }
    let now = pane.borrow().clone();
    assert!(matches!(now.subject, Some(Subject::Outcome(_))), "the linked read is dropped: {now:?}");
    session.stop();
}

/// Back returns to what the pane showed when its place was left: Caused by from outcome A, then outcome B selected on
/// that link's place, Show Actor, Back shows B with its row selected. Back once more shows A, and Forward B again.
#[tokio::test]
async fn back_returns_to_an_outcome_selected_on_a_link_place() {
    let actors = actors().await;
    let rule = routes(vec![
        (
            "/outcomes?limit=100&status=failed",
            page(&[outcome("a1", "bag/a", "failed", Some("p1")), outcome("b1", "bag/b", "failed", None)], None),
        ),
        ("/messages/p1", Fake::Answer(StatusCode::OK, status("p1", "done"))),
        ("/messages/p1/caused?limit=50", page(&[], None)),
    ]);
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let outcomes = session.outcomes();
    let (mut list, mut pane) = (outcomes.list(), outcomes.pane());
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, select_outcomes);
    outcomes.show();
    let view = until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    outcomes.select(Some(&id("a1")));
    session.follow(&mut navigation, id("p1"));
    until(&mut pane, |p| matches!(p.subject, Some(Subject::Linked(_, Read::Found(_))))).await;
    outcomes.select(Some(&id("b1")));
    assert_eq!(shown(&pane.borrow()), Some("b1".into()));
    session.show_actor(&mut navigation, view.rows[1].actor.clone());
    assert_eq!(navigation.current(), Section::Actors);

    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!(navigation.current(), Section::Outcomes);
    let back = pane.borrow().clone();
    assert!(matches!(&back.subject, Some(Subject::Outcome(o)) if Arc::ptr_eq(o, &view.rows[1])), "{back:?}");
    assert_eq!(outcomes.selected(), Some(id("b1")));
    assert!(session.navigate(&mut navigation, Navigation::back));
    assert_eq!((shown(&pane.borrow()), outcomes.selected()), (Some("a1".into()), Some(id("a1"))));
    assert!(session.navigate(&mut navigation, Navigation::forward));
    assert_eq!((shown(&pane.borrow()), outcomes.selected()), (Some("b1".into()), Some(id("b1"))));
    session.stop();
}

/// A linked message that was found shows Caused loading, not "None", until its first page answers: that page is held
/// at the server while the pane is looked at.
#[tokio::test]
async fn a_linked_message_shows_caused_loading_until_its_first_page_answers() {
    let actors = actors().await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held = Fake::Later {
        arrived: arrived.clone(),
        release: release.clone(),
        status: StatusCode::OK,
        body: json!({"items": [], "next": null}),
    };
    let rule = routes(vec![
        ("/messages/p1", Fake::Answer(StatusCode::OK, status("p1", "done"))),
        ("/messages/p1/caused?limit=50", held),
    ]);
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(&server, &dir);
    let mut pane = session.outcomes().pane();
    let mut navigation = Navigation::default();
    session.navigate(&mut navigation, select_outcomes);
    session.follow(&mut navigation, id("p1"));
    timeout(WAIT, arrived.notified()).await.expect("the first page of Caused is held");
    let found = pane.borrow_and_update().clone();
    assert!(matches!(found.subject, Some(Subject::Linked(_, Read::Found(_)))), "{found:?}");
    assert_eq!(found.caused.paging, Paging::Loading);
    assert!(!found.caused.none());
    release.notify_one();
    let answered = until(&mut pane, |p| p.caused.paging == Paging::Idle).await;
    assert!(answered.caused.none());
    session.stop();
}
