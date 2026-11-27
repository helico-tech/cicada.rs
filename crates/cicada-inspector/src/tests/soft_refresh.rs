//! Soft refreshes against a scripted in-process server, with a paused clock and a wall-clock watchdog.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::ActorAddress;
use serde_json::{Value, json};
use tokio::{runtime::Handle, sync::Notify, time::Instant};

use super::messages::{envelope, id};
use super::{
    Fake, HOLD_LIMIT, Rule, actors, config, nodes, outcomes, problem, serve, turns, types, view_until, watches,
};
use crate::messages::Messages;
use crate::model::{Limits, Navigation, Part, Section, Session, TIMED_OUT, View};
use crate::nodes::{Nodes, NodesRead};
use crate::outcomes::{OutcomePlace, Outcomes, Read, StatusFilter, Subject};
use crate::pager::Paging;
use crate::turns::RunningTurns;
use crate::types::Types;
use crate::watches::Watches;

const INBOX: &str = "/messages?limit=100";
const OUTCOMES: &str = "/outcomes?limit=100&status=failed";

async fn with_clock(test: impl Future<Output = ()>) {
    let (release, hold) = std::sync::mpsc::channel();
    let mut watchdog = tokio::task::spawn_blocking(move || hold.recv_timeout(HOLD_LIMIT));
    tokio::select! {
        () = test => {}
        _ = &mut watchdog => panic!("soft refresh test exceeded {HOLD_LIMIT:?}"),
    }
    release.send(()).unwrap();
    watchdog.await.unwrap().unwrap();
}

#[derive(Clone)]
struct Script(Arc<Mutex<HashMap<&'static str, VecDeque<Fake>>>>);

impl Script {
    fn new<const N: usize>(routes: [(&'static str, Vec<Fake>); N]) -> Self {
        Self(Arc::new(Mutex::new(routes.into_iter().map(|(path, steps)| (path, steps.into())).collect())))
    }

    fn rule(&self) -> Rule {
        let routes = self.0.clone();
        Arc::new(move |target| {
            routes
                .lock()
                .unwrap()
                .get_mut(target)
                .map(|steps| steps.pop_front().unwrap_or_else(|| panic!("unexpected request {target}")))
        })
    }

    fn remaining(&self, path: &str) -> usize {
        self.0.lock().unwrap()[path].len()
    }
}

fn page(items: impl IntoIterator<Item = Value>, next: Option<&str>) -> Fake {
    let items: Vec<_> = items.into_iter().collect();
    Fake::Answer(StatusCode::OK, json!({"items": items, "next": next}))
}

fn later(answer: Fake) -> (Fake, Arc<Notify>, Arc<Notify>) {
    let Fake::Answer(status, body) = answer else { unreachable!() };
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    (Fake::Later { arrived: arrived.clone(), release: release.clone(), status, body }, arrived, release)
}

fn messages(range: std::ops::Range<usize>) -> impl Iterator<Item = Value> {
    range.map(|n| envelope(&format!("m{n:03}"), "counter/a", 1))
}

#[tokio::test(start_paused = true)]
async fn messages_refresh_to_three_pages_when_the_old_last_message_was_consumed() {
    with_clock(async {
        let routes = Script::new([
            (INBOX, vec![page(messages(0..100), Some("old1")), page(messages(1..101), Some("new1"))]),
            ("/messages?cursor=old1&limit=100", vec![page(messages(100..200), Some("old2"))]),
            ("/messages?cursor=old2&limit=100", vec![page(messages(200..300), Some("old3"))]),
            ("/messages?cursor=old3&limit=100", vec![page(messages(300..400), Some("old4"))]),
            ("/messages?cursor=old4&limit=100", vec![page(messages(400..450), Some("old5"))]),
            ("/messages?cursor=new1&limit=100", vec![page(messages(101..201), Some("new2"))]),
            ("/messages?cursor=new2&limit=100", vec![page(messages(201..301), Some("new3"))]),
            ("/messages?cursor=new3&limit=100", vec![page(messages(301..401), Some("new4"))]),
            ("/messages?cursor=new4&limit=100", vec![page(messages(401..449).chain(messages(450..451)), None)]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Messages::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let first = view_until(&mut list, |list| list.rows.len() == 100).await;
        for count in [200, 300, 400, 450] {
            model.more();
            view_until(&mut list, |list| list.rows.len() == count).await;
        }
        model.select(Some(&id("m002")));
        model.soft_refresh();
        let after = view_until(&mut list, |list| list.stale.is_some()).await;
        assert_eq!(after.generation, first.generation);
        let mark = after.stale.as_ref().unwrap();
        assert!(mark.capped);
        assert!(mark.text().starts_with("rows past 300 as of "));
        assert!(mark.error.is_none());
        assert_eq!(after.rows.len(), 449, "m300 appeared again at the seam");
        assert_eq!(after.index(&id("m000")), None);
        assert!(after.index(&id("m449")).is_some(), "the carried-over last message was consumed");
        assert_eq!(model.selected(), Some(id("m002")));
        assert_eq!(routes.remaining("/messages?cursor=new3&limit=100"), 1);
        model.more();
        let after = view_until(&mut list, |list| !list.more).await;
        assert_eq!(after.rows.len(), 450, "pages reached from the new cursor add only unseen ids");
        assert!(after.index(&id("m450")).is_some());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn messages_refresh_keeps_old_rows_pushed_back_by_new_ones_ahead_of_them() {
    with_clock(async {
        let fresh = (0..5).map(|n| envelope(&format!("a{n:03}"), "counter/0", 1)).chain(messages(0..95));
        let routes = Script::new([
            (INBOX, vec![page(messages(0..100), Some("old1")), page(fresh, Some("new1"))]),
            ("/messages?cursor=old1&limit=100", vec![page(messages(100..200), Some("old2"))]),
            ("/messages?cursor=old2&limit=100", vec![page(messages(200..300), Some("old3"))]),
            ("/messages?cursor=old3&limit=100", vec![page(messages(300..400), Some("old4"))]),
            ("/messages?cursor=old4&limit=100", vec![page(messages(400..450), Some("old5"))]),
            ("/messages?cursor=new1&limit=100", vec![page(messages(95..195), Some("new2"))]),
            ("/messages?cursor=new2&limit=100", vec![page(messages(195..295), Some("new3"))]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Messages::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        view_until(&mut list, |list| list.rows.len() == 100).await;
        for count in [200, 300, 400, 450] {
            model.more();
            view_until(&mut list, |list| list.rows.len() == count).await;
        }
        model.soft_refresh();
        let after = view_until(&mut list, |list| list.stale.is_some()).await;
        assert_eq!(after.rows.len(), 455, "five new rows and every old one");
        for n in 295..300 {
            assert!(after.index(&id(&format!("m{n:03}"))).is_some(), "m{n:03} was pushed past the range and lost");
        }
        assert_eq!(after.index(&id("m295")), Some(300), "the old tail follows the refreshed range");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn messages_refresh_does_not_let_a_row_moved_ahead_set_the_seam() {
    with_clock(async {
        // Deliver Now gives m440 a new place in delivery order: here a lower `seq` puts it first.
        let mut moved = envelope("m440", "counter/a", 1);
        moved["seq"] = json!(0);
        let fresh = std::iter::once(moved).chain(messages(0..99));
        let routes = Script::new([
            (INBOX, vec![page(messages(0..100), Some("old1")), page(fresh, Some("new1"))]),
            ("/messages?cursor=old1&limit=100", vec![page(messages(100..200), Some("old2"))]),
            ("/messages?cursor=old2&limit=100", vec![page(messages(200..300), Some("old3"))]),
            ("/messages?cursor=old3&limit=100", vec![page(messages(300..400), Some("old4"))]),
            ("/messages?cursor=old4&limit=100", vec![page(messages(400..450), Some("old5"))]),
            ("/messages?cursor=new1&limit=100", vec![page(messages(99..199), Some("new2"))]),
            ("/messages?cursor=new2&limit=100", vec![page(messages(199..299), Some("new3"))]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Messages::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        view_until(&mut list, |list| list.rows.len() == 100).await;
        for count in [200, 300, 400, 450] {
            model.more();
            view_until(&mut list, |list| list.rows.len() == count).await;
        }
        model.soft_refresh();
        let after = view_until(&mut list, |list| list.stale.is_some()).await;
        assert_eq!(after.rows.len(), 450, "m440 delivered now, every row kept once");
        assert_eq!(after.index(&id("m440")), Some(0));
        assert_eq!(after.index(&id("m299")), Some(300), "the seam follows m298, not m440");
        assert_eq!(after.index(&id("m449")), Some(449));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn messages_refresh_stops_at_the_old_last_place_and_uses_the_selection_at_apply_time() {
    with_clock(async {
        let (held, arrived, release) =
            later(page([envelope("m2", "counter/a", 9), envelope("m3", "counter/a", 2)], Some("new")));
        let routes = Script::new([
            (
                INBOX,
                vec![
                    page((1..=3).map(|n| envelope(&format!("m{n}"), "counter/a", 1)), None),
                    held,
                    page([envelope("m3", "counter/a", 3)], None),
                ],
            ),
            ("/messages?cursor=new&limit=100", vec![page([], None)]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Messages::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        view_until(&mut list, |list| list.rows.len() == 3).await;
        model.select(Some(&id("m1")));
        model.soft_refresh();
        arrived.notified().await;
        model.select(Some(&id("m2")));
        release.notify_one();
        let shown = view_until(&mut pane, |pane| pane.message.as_ref().is_some_and(|row| row.attempt == 9)).await;
        assert_eq!(shown.message.unwrap().id, id("m2"));
        assert_eq!(model.selected(), Some(id("m2")));
        assert!(list.borrow().index(&id("m1")).is_none());
        assert!(list.borrow().stale.is_none());
        assert_eq!(routes.remaining("/messages?cursor=new&limit=100"), 1, "the old last id was reached");
        model.soft_refresh();
        view_until(&mut list, |list| list.rows.len() == 1).await;
        view_until(&mut pane, |pane| pane.message.is_none()).await;
        assert!(model.selected().is_none());
        assert!(!list.borrow().more);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn running_turns_refresh_reaches_the_old_key_then_continues_from_the_fresh_cursor() {
    with_clock(async {
        let first = "/turns?limit=100";
        let new_tail = "/turns?cursor=new2&limit=100";
        let routes = Script::new([
            (
                first,
                vec![
                    page([turns::turn("counter/a", "a"), turns::turn("counter/c", "c")], Some("old")),
                    page([turns::turn("counter/b", "b")], Some("new1")),
                    page([turns::turn("counter/e", "last")], None),
                ],
            ),
            ("/turns?cursor=old&limit=100", vec![page([turns::turn("counter/d", "d")], None)]),
            (
                "/turns?cursor=new1&limit=100",
                vec![page([turns::turn("counter/c", "new-c"), turns::turn("counter/e", "e")], Some("new2"))],
            ),
            (new_tail, vec![page([turns::turn("counter/f", "f")], None)]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = RunningTurns::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let (mut list, mut pane) = (model.list(), model.pane());
        let address = "counter/c".parse::<ActorAddress>().unwrap();
        model.show();
        let first = view_until(&mut list, |list| list.rows.len() == 2).await;
        model.more();
        view_until(&mut list, |list| list.rows.len() == 3).await;
        model.select(Some(&address));
        model.soft_refresh();
        let after = view_until(&mut list, |list| {
            list.more && list.rows.first().is_some_and(|row| row.actor.as_str() == "counter/b")
        })
        .await;
        assert_eq!(after.generation, first.generation);
        assert_eq!(
            after.rows.iter().map(|row| row.actor.as_str()).collect::<Vec<_>>(),
            ["counter/b", "counter/c", "counter/e"]
        );
        assert!(after.stale.is_none());
        assert_eq!(routes.remaining(new_tail), 1);
        let shown =
            view_until(&mut pane, |pane| pane.turn.as_ref().is_some_and(|row| row.message_id == id("new-c"))).await;
        assert_eq!(shown.selected, Some(address));
        model.more();
        view_until(&mut list, |list| list.rows.len() == 4 && !list.more).await;
        model.soft_refresh();
        view_until(&mut list, |list| list.rows.len() == 1 && !list.more).await;
        view_until(&mut pane, |pane| pane.turn.is_none()).await;
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn watches_refreshes_hidden_rows_then_merges_the_ordered_tail_without_duplicates() {
    with_clock(async {
        let row =
            |n: usize| watches::row(&format!("counter/{n:03}"), if n.is_multiple_of(2) { "sync/a" } else { "bag/a" });
        let routes = Script::new([
            (
                "/watches?limit=100",
                vec![
                    page((0..100).map(row), Some("old2")),
                    page((1..101).map(row), Some("new2")),
                    page([row(10)], None),
                ],
            ),
            ("/watches?cursor=old2&limit=100", vec![page((100..200).map(row), Some("old3"))]),
            ("/watches?cursor=old3&limit=100", vec![page((200..300).map(row), Some("old4"))]),
            ("/watches?cursor=old4&limit=100", vec![page((300..399).chain([420]).map(row), None)]),
            ("/watches?cursor=new2&limit=100", vec![page((101..201).map(row), Some("new3"))]),
            ("/watches?cursor=new3&limit=100", vec![page((201..301).map(row), Some("new4"))]),
            ("/watches?cursor=new4&limit=100", vec![page((301..398).chain([400, 410, 420]).map(row), None)]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Watches::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        for count in [100, 200, 300] {
            view_until(&mut list, |list| list.loaded == count).await;
            model.more();
        }
        view_until(&mut list, |list| list.loaded == 400).await;
        model.search("sync/");
        let selected = list.borrow().rows[5].clone();
        model.select(Some(&selected));
        model.soft_refresh();
        let partial = view_until(&mut list, |list| list.stale.is_some()).await;
        assert_eq!(partial.loaded, 399);
        assert!(partial.stale.unwrap().capped);
        assert!(partial.rows.iter().all(|row| row.watcher.as_str() == "sync/a"));
        view_until(&mut pane, |pane| pane.as_deref() == Some(selected.as_ref())).await;
        model.more();
        let paged = view_until(&mut list, |list| list.loaded == 401 && !list.more).await;
        assert!(paged.rows.windows(2).all(|pair| pair[0].target.as_str() < pair[1].target.as_str()));
        model.soft_refresh();
        let complete = view_until(&mut list, |list| list.loaded == 1).await;
        assert!(complete.stale.is_none() && !complete.more);
        assert_eq!(complete.rows[0].target.as_str(), "counter/010");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn outcomes_refreshes_rows_and_rereads_a_followed_link_without_loading() {
    with_clock(async {
        let outcome = |id| outcomes::outcome(id, "counter/a", "failed", None);
        let mut updated = outcome("m2");
        updated["error"] = json!("new error");
        let (linked, arrived, release) = later(Fake::Answer(StatusCode::OK, outcomes::status("link", "done")));
        let routes = Script::new([
            (OUTCOMES, vec![page([outcome("m1")], Some("old")), page([updated], Some("new")), page([], None)]),
            ("/outcomes?cursor=old&limit=100&status=failed", vec![page([outcome("m2")], None)]),
            ("/outcomes?cursor=new&limit=100&status=failed", vec![page([outcome("m3")], None)]),
            ("/messages/m2/caused?limit=50", vec![page([], None)]),
            ("/messages/link", vec![Fake::Answer(StatusCode::OK, outcomes::status("link", "pending")), linked]),
            ("/messages/link/caused?limit=50", vec![page([], None)]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Outcomes::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        let before = view_until(&mut list, |list| list.rows.len() == 1).await;
        model.more();
        view_until(&mut list, |list| list.rows.len() == 2).await;
        model.select(Some(&id("m2")));
        view_until(&mut pane, |pane| pane.caused.paging == Paging::Idle).await;
        model.soft_refresh();
        let refreshed = view_until(&mut list, |list| list.more).await;
        assert_eq!(refreshed.generation, before.generation);
        assert_eq!(refreshed.rows.len(), 1);
        assert!(refreshed.index(&id("m1")).is_none());
        assert_eq!(model.selected(), Some(id("m2")));
        view_until(&mut pane, |pane| {
            pane.subject.as_ref().and_then(Subject::details).and_then(|details| details.error) == Some("new error")
        })
        .await;
        model.more();
        view_until(&mut list, |list| list.index(&id("m3")).is_some()).await;
        model.restore(OutcomePlace::Linked(id("link")));
        let shown = view_until(&mut pane, |pane| {
            matches!(pane.subject, Some(Subject::Linked(_, Read::Found(_)))) && pane.caused.paging == Paging::Idle
        })
        .await;
        model.soft_refresh();
        arrived.notified().await;
        assert_eq!(*pane.borrow(), shown);
        release.notify_one();
        let new = view_until(&mut pane, |pane| {
            pane.subject.as_ref().and_then(Subject::details).is_some_and(|detail| detail.finished_at.is_some())
        })
        .await;
        assert_eq!(new.generation, shown.generation);
        assert!(list.borrow().rows.is_empty() && list.borrow().stale.is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn soft_refresh_waits_for_paging_skips_a_failure_and_remembers_one_request_during_retry() {
    with_clock(async {
        let (failure, failed_request, fail) = later(problem(503));
        let (page_retry, retry_started, retry_finish) = later(page([envelope("m2", "counter/a", 1)], None));
        let (soft, soft_started, soft_finish) = later(page([envelope("m2", "counter/a", 2)], None));
        let tail = "/messages?cursor=retry&limit=100";
        let routes = Script::new([
            (
                INBOX,
                vec![
                    page([envelope("m1", "counter/a", 1)], Some("retry")),
                    soft,
                    page([envelope("m2", "counter/a", 3)], None),
                ],
            ),
            (tail, vec![failure, page_retry]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Messages::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let first = view_until(&mut list, |list| list.rows.len() == 1).await;
        model.more();
        failed_request.notified().await;
        model.soft_refresh();
        assert_eq!(list.borrow().paging, Paging::Loading);
        fail.notify_one();
        view_until(&mut list, |list| matches!(list.paging, Paging::Failed(_))).await;
        model.soft_refresh();
        model.retry();
        retry_started.notified().await;
        assert_eq!(routes.remaining(INBOX), 2);
        model.soft_refresh();
        model.soft_refresh();
        assert_eq!(list.borrow().paging, Paging::Loading);
        assert_eq!(list.borrow().generation, first.generation);
        retry_finish.notify_one();
        soft_started.notified().await;
        assert!(list.borrow().index(&id("m2")).is_some(), "the awaited retry landed in the original generation");
        assert_eq!(routes.remaining(INBOX), 1);
        model.soft_refresh();
        model.soft_refresh();
        soft_finish.notify_one();
        let refreshed = view_until(&mut list, |list| list.rows.iter().any(|row| row.attempt == 3)).await;
        assert_eq!(refreshed.generation, first.generation);
        assert_eq!(refreshed.paging, Paging::Idle);
        assert!(refreshed.stale.is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn soft_refresh_deadline_applies_the_pages_that_answered_and_keeps_their_cursor() {
    with_clock(async {
        let late = "/messages?cursor=fresh&limit=100";
        let routes = Script::new([
            (
                INBOX,
                vec![
                    page([envelope("gone", "counter/a", 1), envelope("tail", "counter/a", 1)], None),
                    page([envelope("fresh", "counter/a", 1)], Some("fresh")),
                    problem(503),
                ],
            ),
            (late, vec![Fake::Hang, page([envelope("next", "counter/a", 1)], None)]),
        ]);
        let mut server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Messages::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let first = view_until(&mut list, |list| list.rows.len() == 2).await;
        let start = Instant::now();
        model.soft_refresh();
        let wanted = format!("arrived {late}");
        while server.events.recv().await.unwrap() != wanted {}
        tokio::time::advance(Duration::from_secs(9)).await;
        assert_eq!(list.borrow().rows, first.rows);
        tokio::time::advance(Duration::from_secs(1)).await;
        let partial = view_until(&mut list, |list| list.stale.is_some()).await;
        assert_eq!(Instant::now(), start + Duration::from_secs(10));
        assert_eq!(partial.stale.unwrap().error.as_deref(), Some(TIMED_OUT));
        // The range holds no old row, so nothing tells which old rows left the list: all are kept, not refreshed.
        assert_eq!(partial.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["fresh", "gone", "tail"]);
        model.more();
        let paged = view_until(&mut list, |list| !list.more).await;
        assert!(paged.index(&id("next")).is_some());
        model.soft_refresh();
        let failed = view_until(&mut list, |list| {
            list.stale.as_ref().and_then(|mark| mark.error.as_deref()).is_some_and(|error| error != TIMED_OUT)
        })
        .await;
        assert_eq!(failed.rows, paged.rows);
        assert!(!failed.more);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn soft_refresh_cancels_on_refresh_prefix_status_page_and_context() {
    with_clock(async {
        let (held, refresh_started, release_refresh) = later(page([envelope("old-refresh", "bag/a", 1)], None));
        let (after_refresh, prefix_started, release_prefix) = later(page([envelope("old-prefix", "bag/a", 1)], None));
        let (after_prefix, page_started, release_page) = later(page([envelope("old-page", "counter/a", 1)], None));
        let (after_page, context_started, release_context) =
            later(page([envelope("old-context", "counter/a", 1)], None));
        let (status, status_started, release_status) =
            later(page([outcomes::outcome("old-status", "counter/a", "failed", None)], None));
        let (before, after) = (actors().await, actors().await);
        let routes = Script::new([
            (
                INBOX,
                vec![
                    page([envelope("start", "bag/a", 1)], None),
                    held,
                    page([envelope("hard", "bag/a", 1)], None),
                    after_refresh,
                ],
            ),
            (
                "/messages?prefix=counter%2F&limit=100",
                vec![page([envelope("new-prefix", "counter/a", 1)], None), after_prefix, after_page],
            ),
            (OUTCOMES, vec![page([], None), status]),
            (
                "/outcomes?limit=100&status=done",
                vec![page([outcomes::outcome("done", "counter/a", "done", None)], None)],
            ),
        ]);
        let next = Script::new([(
            "/messages?prefix=counter%2F&limit=100",
            vec![page([envelope("new-context", "counter/a", 1)], None)],
        )]);
        let before = serve(&before, routes.rule(), Duration::ZERO).await;
        let after = serve(&after, next.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let env = config(&dir, &[("before", &before.url), ("after", &after.url)]);
        let session = Session::new(env, Handle::current(), Limits { debounce: Duration::ZERO, ..Limits::DEFAULT });
        let model = session.messages();
        let mut list = model.list();
        let mut nav = Navigation::default();
        session.use_context(Some("before".into()));
        session.navigate(&mut nav, |n| {
            n.select(Section::Messages);
            true
        });
        model.show();
        let first = view_until(&mut list, |list| list.index(&id("start")).is_some()).await;

        session.soft_refresh(Section::Messages, None);
        refresh_started.notified().await;
        model.refresh();
        let hard = view_until(&mut list, |list| list.index(&id("hard")).is_some()).await;
        release_refresh.notify_one();
        assert!(hard.generation > first.generation && hard.stale.is_none());

        session.soft_refresh(Section::Messages, None);
        prefix_started.notified().await;
        model.search("counter/");
        tokio::time::advance(Duration::from_millis(1)).await;
        let searched = view_until(&mut list, |list| list.index(&id("new-prefix")).is_some()).await;
        release_prefix.notify_one();
        assert!(searched.generation > hard.generation);

        session.soft_refresh(Section::Messages, None);
        page_started.notified().await;
        session.navigate(&mut nav, |n| {
            n.select(Section::Outcomes);
            true
        });
        let outcomes = session.outcomes();
        let mut outcomes_list = outcomes.list();
        outcomes.show();
        view_until(&mut outcomes_list, |list| list.paging == Paging::Idle).await;
        release_page.notify_one();
        session.soft_refresh(Section::Outcomes, None);
        status_started.notified().await;
        outcomes.set_status(StatusFilter::Done);
        view_until(&mut outcomes_list, |list| list.index(&id("done")).is_some()).await;
        release_status.notify_one();
        assert_eq!(list.borrow().rows, searched.rows, "a finished read of the old page was discarded");

        session.navigate(&mut nav, |n| {
            n.select(Section::Messages);
            true
        });
        session.soft_refresh(Section::Messages, None);
        context_started.notified().await;
        session.switch_context(&mut nav, Some("after".into()));
        model.show();
        let switched = view_until(&mut list, |list| list.index(&id("new-context")).is_some()).await;
        release_context.notify_one();
        assert!(switched.generation > searched.generation);
        assert!(list.borrow().index(&id("old-context")).is_none());
        assert!(outcomes_list.borrow().rows.is_empty());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn nodes_soft_refresh_keeps_the_pane_until_the_fresh_read_and_marks_a_failed_list() {
    with_clock(async {
        let (detail, arrived, release) = later(nodes::detail("n1", "fresh", &["counter/a"]));
        let echo = Fake::Answer(StatusCode::SERVICE_UNAVAILABLE, json!({
            "type": "about:blank", "status": 503, "title": "Unavailable", "detail": "Authorization: Bearer test-secret"
        }));
        let routes = Script::new([
            (
                "/nodes",
                vec![
                    nodes::nodes_answer(&[nodes::node("n1", "alive"), nodes::node("n2", "suspect")]),
                    nodes::nodes_answer(&[nodes::node("n1", "dead")]),
                    echo,
                    problem(503),
                ],
            ),
            ("/nodes/n1", vec![nodes::detail("n1", "old", &[]), detail]),
        ]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Nodes::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        let before = view_until(&mut list, |list| list.loaded == 2).await;
        model.select(Some(&nodes::node_id("n1")));
        let old = view_until(&mut pane, |pane| nodes::label(pane).as_deref() == Some("old")).await;
        model.soft_refresh();
        arrived.notified().await;
        assert_eq!(*pane.borrow(), old);
        assert_eq!(list.borrow().generation, before.generation);
        assert_eq!(list.borrow().loaded, 1);
        release.notify_one();
        let fresh = view_until(&mut pane, |pane| nodes::label(pane).as_deref() == Some("fresh")).await;
        assert_eq!(fresh.generation, old.generation);
        model.soft_refresh();
        let stale = view_until(&mut list, |list| list.stale.is_some()).await;
        assert_eq!(stale.rows.len(), 1);
        assert_eq!(stale.stale.unwrap().error.as_deref(), Some("HTTP 503"));
        assert_eq!(*pane.borrow(), fresh);
        model.refresh();
        let failed = view_until(&mut list, |list| matches!(list.read, NodesRead::Failed(_))).await;
        assert!(failed.rows.is_empty() && failed.stale.is_none() && pane.borrow().node.is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn types_soft_refresh_keeps_the_schema_by_name_and_keeps_its_json_during_formatting() {
    with_clock(async {
        let mut changed = types::registered("counter");
        changed["messages"] = json!({"Aardvark": {}, "Alpha": {"new": true}});
        let routes = Script::new([(
            "/types",
            vec![page([types::registered("counter")], None), page([changed], None), problem(503), page([], None)],
        )]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Types::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        view_until(&mut list, |list| list.rows.len() == 1).await;
        model.select(Some("counter"));
        model.choose(1);
        model.set_pane_visible(true);
        let before = view_until(&mut pane, |pane| pane.json.is_some()).await;
        let (formatting, release) = model.hold_format();
        model.soft_refresh();
        tokio::task::spawn_blocking(move || formatting.recv_timeout(HOLD_LIMIT)).await.unwrap().unwrap();
        let during = pane.borrow().clone();
        assert_eq!(during.schemas(), ["State", "Aardvark", "Alpha"]);
        assert_eq!(during.schema, 2);
        assert_eq!(during.json, before.json);
        release.send(()).unwrap();
        let fresh = view_until(&mut pane, |pane| pane.json != before.json).await;
        assert!(fresh.json.is_some());
        model.soft_refresh();
        let stale = view_until(&mut list, |list| list.stale.is_some()).await;
        assert_eq!(stale.rows[0].name, "counter");
        assert_eq!(pane.borrow().json, fresh.json);
        model.soft_refresh();
        view_until(&mut pane, |pane| pane.row.is_none()).await;
        assert!(list.borrow().rows.is_empty() && list.borrow().stale.is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn types_soft_refresh_cancelled_while_formatting_puts_the_pane_back() {
    with_clock(async {
        let mut changed = types::registered("counter");
        changed["messages"] = json!({"Aardvark": {}, "Alpha": {"new": true}});
        let routes = Script::new([("/types", vec![page([types::registered("counter")], None), page([changed], None)])]);
        let server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let model = Types::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        view_until(&mut list, |list| list.rows.len() == 1).await;
        model.select(Some("counter"));
        model.choose(1);
        model.set_pane_visible(true);
        let before = view_until(&mut pane, |pane| pane.json.is_some()).await;
        let formatted = model.formatted();
        let (formatting, release) = model.hold_format();
        model.soft_refresh();
        tokio::task::spawn_blocking(move || formatting.recv_timeout(HOLD_LIMIT)).await.unwrap().unwrap();
        assert_eq!(pane.borrow().schemas(), ["State", "Aardvark", "Alpha"]);

        // A pause cancels the refresh while its schema is being formatted: none of its pane applies.
        model.cancel_soft_refresh();
        release.send(()).unwrap();
        let after = view_until(&mut pane, |pane| pane.schemas() == before.schemas()).await;
        assert_eq!((after.schema, &after.json), (before.schema, &before.json));
        assert_eq!(model.formatted(), formatted + 1, "the refresh's formatting finished and was dropped");
    })
    .await;
}

fn stats(actors: u64) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"actors": actors, "pending": 0, "failed": 2}))
}

#[tokio::test(start_paused = true)]
async fn overview_soft_refresh_shares_four_slots_and_keeps_the_part_that_misses_the_deadline() {
    with_clock(async {
        let (held_stats, stats_in, stats_done) = later(stats(42));
        let (held_health, health_in, health_done) = later(Fake::Answer(StatusCode::OK, json!({"status":"ok"})));
        let old_nodes = nodes::nodes_answer(&[nodes::node("n1", "alive")]);
        let (held_turns, turns_in, turns_done) = later(page([], None));
        let routes = Script::new([
            ("/stats", vec![stats(1), held_stats]),
            ("/health", vec![Fake::Answer(StatusCode::OK, json!({"status":"ok"})), held_health]),
            ("/nodes", vec![old_nodes, Fake::Hang]),
            ("/turns?limit=20", vec![page([], None), held_turns]),
            ("/types", vec![page([], None), page([types::registered("counter"), types::registered("slow")], None)]),
            ("/stats?prefix=counter%2F", vec![stats(7)]),
            ("/stats?prefix=slow%2F", vec![Fake::Hang]),
        ]);
        let mut server = serve(&actors().await, routes.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let limits = Limits { concurrent: 1, ..Limits::DEFAULT };
        let session = Session::new(config(&dir, &[("local", &server.url)]), Handle::current(), limits);
        let mut view = session.view();
        session.use_context(None);
        let before = view_until(&mut view, |view| view.overview.is_some()).await;
        let start = Instant::now();
        session.soft_refresh(Section::Overview, None);
        stats_in.notified().await;
        health_in.notified().await;
        turns_in.notified().await;
        while server.events.recv().await.unwrap() != "arrived /nodes" {}
        assert_eq!(routes.remaining("/types"), 1);
        stats_done.notify_one();
        // With one count at a time, arrival of the stalled count proves the first count was received.
        while server.events.recv().await.unwrap() != "arrived /stats?prefix=slow%2F" {}
        health_done.notify_one();
        turns_done.notify_one();
        assert!(view.borrow().stale.is_none());
        tokio::time::advance(Duration::from_secs(10)).await;
        let after: View = view_until(&mut view, |view| view.stale.is_some()).await;
        assert_eq!(Instant::now(), start + Duration::from_secs(10));
        assert_eq!(after.stale.unwrap().error.as_deref(), Some(TIMED_OUT));
        let (after, before) = (after.overview.unwrap(), before.overview.unwrap());
        assert_eq!(after.nodes, before.nodes);
        assert!(matches!(after.stats, Part::Ready(stats) if stats.actors == 42));
        assert!(matches!(after.types, Part::Ready(types) if types[0].actors == crate::model::Count::Counted(7)));
    })
    .await;
}

mod actor_lists;
mod live;
