//! The live model on the session's pages, against a scripted server and a channel change source.

use std::sync::Weak;

use cicada_client::Change;
use tokio::sync::{mpsc, watch};

use super::*;
use crate::actors::{ActorPath, ListView, OutlineViewport, PaneView, Read as ActorRead};
use crate::live::channel::{Channel, Opened};
use crate::live::{FALLBACK, GATHER, Live, Pages};

const ROOTS: &str = "/actors/states?limit=100";
const SAMPLE: &str = "/turns?limit=200";
const DETAILS: &str = "/actors/counter/a";
const TURN: &str = "/actors/counter/a/turn";
const MAILBOX: &str = "/actors/counter/a/mailbox?limit=50";
const WATCHING: &str = "/watches?limit=50&watcher=counter%2Fa";
const WATCHED_BY: &str = "/watches?limit=50&target=counter%2Fa";

fn roots(keys: &[&str]) -> Fake {
    page(
        keys.iter().map(|key| {
            json!({
                "address": format!("counter/{key}"), "type": "counter", "version": 1,
                "createdAt": "2026-11-27T10:00:00.000Z", "paused": false, "state": {}
            })
        }),
        None,
    )
}

fn details(version: u64) -> Fake {
    Fake::Answer(
        StatusCode::OK,
        json!({
            "address": "counter/a", "type": "counter", "createdAt": "2026-11-27T10:00:00.000Z",
            "paused": false, "version": version, "state": {"version": version}
        }),
    )
}

fn root(key: &str) -> ActorPath {
    ActorPath::root(ActorAddress::parse(&format!("counter/{key}")).unwrap())
}

fn version(pane: &PaneView) -> Option<i64> {
    match &pane.actor {
        ActorRead::Ready(Some(actor)) => Some(actor.version),
        _ => None,
    }
}

/// A session on `server` showing `section`, and a live model of it on a channel source with its first stream.
async fn live_session(
    url: &str,
    dir: &tempfile::TempDir,
    section: Section,
) -> (Arc<Session>, Navigation, Live<Channel>, mpsc::UnboundedReceiver<Opened>) {
    let limits = Limits { debounce: Duration::ZERO, ..Limits::DEFAULT };
    let session = Arc::new(Session::new(config(dir, &[("local", url)]), Handle::current(), limits));
    let mut nav = Navigation::default();
    session.use_context(None);
    session.navigate(&mut nav, |n| {
        n.select(section);
        true
    });
    let pages: Arc<dyn Pages> = session.clone();
    let weak: Weak<dyn Pages> = Arc::downgrade(&pages);
    let live = Live::new(&Handle::current(), weak, Duration::ZERO);
    let (source, opened) = Channel::new();
    live.show(section, "");
    live.use_source(Some(source));
    (session, nav, live, opened)
}

/// Lets the live model's task handle what was sent or what is due before the clock moves on.
async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

/// Overview reads of the session's context switch go to the real API; this waits for them to end.
async fn overview_read(session: &Session) {
    let mut view = session.view();
    view_until(&mut view, |view| view.overview.is_some()).await;
}

#[tokio::test(start_paused = true)]
async fn the_timer_recovers_a_page_after_retry_without_hints() {
    with_clock(async {
        let script = Script::new([
            (ROOTS, vec![problem(503), roots(&["a"]), roots(&["a", "b"])]),
            (SAMPLE, vec![page([], None), page([], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let (session, _nav, _live, mut opened) = live_session(&server.url, &dir, Section::Actors).await;
        overview_read(&session).await;
        let model = session.actors();
        let mut list = model.list();
        model.show();
        view_until(&mut list, |list| matches!(list.paging, Paging::Failed(_))).await;
        let stream = opened.recv().await.unwrap();

        // The timer's attempt is skipped while paging has failed, and armed again.
        tokio::time::advance(FALLBACK + Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(script.remaining(ROOTS), 2);
        model.retry();
        view_until(&mut list, |list| list.rows.len() == 1).await;
        tokio::time::advance(FALLBACK).await;
        let refreshed: ListView = view_until(&mut list, |list| list.rows.len() == 2).await;
        assert!(refreshed.footer().is_none(), "nothing is marked");
        assert!(!stream.hints.is_closed(), "no hint and no reconnect was needed");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_hint_for_the_selected_actor_refreshes_its_pane_and_keeps_the_selection() {
    with_clock(async {
        let (fresh, fresh_in, fresh_out) = later(details(3));
        let three = |fake: Fake| vec![fake.clone(), fake.clone(), fake];
        let script = Script::new([
            (ROOTS, three(roots(&["a", "b"]))),
            (SAMPLE, three(page([], None))),
            (DETAILS, vec![details(1), details(2), fresh]),
            (TURN, three(Fake::Answer(StatusCode::OK, turns::turn("counter/a", "m")))),
            (MAILBOX, three(page([], None))),
            (WATCHING, three(page([], None))),
            (WATCHED_BY, three(page([], None))),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let (session, _nav, _live, mut opened) = live_session(&server.url, &dir, Section::Actors).await;
        overview_read(&session).await;
        let model = session.actors();
        let (mut list, mut pane): (watch::Receiver<ListView>, _) = (model.list(), model.pane());
        model.show();
        let shown = view_until(&mut list, |list| list.rows.len() == 2).await;
        model.select(Some(root("a")));
        view_until(&mut pane, |pane| version(pane) == Some(1)).await;
        let stream = opened.recv().await.unwrap();

        // The first resync refreshes the page, and its pane, once.
        stream.hints.send(Ok(Change::Resync)).unwrap();
        settle().await;
        tokio::time::advance(GATHER).await;
        let resynced = view_until(&mut pane, |pane| version(pane) == Some(2)).await;

        // A hint for the selected actor starts a soft refresh within a second.
        let sent = Instant::now();
        stream.hints.send(Ok(Change::Changed(root("a").address().clone()))).unwrap();
        settle().await;
        tokio::time::advance(GATHER).await;
        fresh_in.notified().await;
        assert!(sent.elapsed() < Duration::from_secs(1), "started after {:?}", sent.elapsed());
        assert_eq!(version(&pane.borrow()), Some(2), "no Loading while the pane is read again");
        fresh_out.notify_one();
        let after = view_until(&mut pane, |pane| version(pane) == Some(3)).await;
        assert_eq!(after.generation, resynced.generation);
        assert_eq!(model.selected(), Some(root("a")));
        assert_eq!(list.borrow().generation, shown.generation, "the list kept its generation and rows");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_timer_catches_an_actors_child_outside_the_prefix() {
    with_clock(async {
        const CHILDREN: &str = "/actors/counter/a/children?limit=100";
        let script = Script::new([
            (ROOTS, vec![roots(&["a"]), roots(&["a"])]),
            (SAMPLE, vec![page([], None), page([], None)]),
            (CHILDREN, vec![page([json!("sync/x")], None), page([json!("sync/x"), json!("sync/y")], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let (session, _nav, live, _opened) = live_session(&server.url, &dir, Section::Actors).await;
        overview_read(&session).await;
        let model = session.actors();
        let mut list = model.list();
        model.show();
        let generation = view_until(&mut list, |list| list.rows.len() == 1).await.generation;
        let a = root("a");
        model.open(generation, &a);
        view_until(&mut list, |list| list.children.get(&a).is_some_and(|page| page.rows.len() == 1)).await;
        model.set_viewport(OutlineViewport { generation, visible: 0..2, open: vec![(0, a.clone())] });
        // The stream of the prefix `counter/` cannot carry a hint for `sync/y`.
        live.show(Section::Actors, "counter/");
        settle().await;
        tokio::time::advance(FALLBACK + Duration::from_secs(1)).await;
        let view = view_until(&mut list, |list| list.children[&a].rows.len() == 2).await;
        assert!(view.open.contains(&a) && !view.children[&a].not_refreshed());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_timer_catches_an_outcomes_link_to_a_message_elsewhere() {
    with_clock(async {
        let script = Script::new([
            (OUTCOMES, vec![page([], None), page([], None)]),
            (
                "/messages/link",
                vec![
                    Fake::Answer(StatusCode::OK, outcomes::status("link", "pending")),
                    Fake::Answer(StatusCode::OK, outcomes::status("link", "done")),
                ],
            ),
            ("/messages/link/caused?limit=50", vec![page([], None), page([], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let (session, _nav, _live, _opened) = live_session(&server.url, &dir, Section::Outcomes).await;
        overview_read(&session).await;
        let model = session.outcomes();
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        view_until(&mut list, |list| list.paging == Paging::Idle).await;
        model.restore(OutcomePlace::Linked(id("link")));
        let shown = view_until(&mut pane, |pane| {
            matches!(pane.subject, Some(Subject::Linked(_, Read::Found(_)))) && pane.caused.paging == Paging::Idle
        })
        .await;
        tokio::time::advance(FALLBACK + Duration::from_secs(1)).await;
        let new = view_until(&mut pane, |pane| {
            pane.subject.as_ref().and_then(Subject::details).is_some_and(|detail| detail.finished_at.is_some())
        })
        .await;
        assert_eq!(new.generation, shown.generation);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn pause_fences_an_actors_refresh_in_flight_and_marks_the_rows_it_dropped() {
    with_clock(async {
        const CHILDREN: &str = "/actors/counter/a/children?limit=100";
        let (fresh_roots, roots_in, roots_out) = later(roots(&["a", "b"]));
        let (fresh_actor, actor_in, actor_out) = later(details(2));
        let two = |fake: Fake| vec![fake.clone(), fake];
        let script = Script::new([
            (ROOTS, vec![roots(&["a"]), fresh_roots]),
            (SAMPLE, two(page([], None))),
            (CHILDREN, two(page([json!("counter/x")], None))),
            (DETAILS, vec![details(1), fresh_actor]),
            (TURN, two(Fake::Answer(StatusCode::OK, turns::turn("counter/a", "m")))),
            (MAILBOX, two(page([], None))),
            (WATCHING, two(page([], None))),
            (WATCHED_BY, two(page([], None))),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let (session, _nav, live, mut opened) = live_session(&server.url, &dir, Section::Actors).await;
        overview_read(&session).await;
        let model = session.actors();
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        let generation = view_until(&mut list, |list| list.rows.len() == 1).await.generation;
        let a = root("a");
        model.open(generation, &a);
        view_until(&mut list, |list| list.children.get(&a).is_some_and(|page| page.paging == Paging::Idle)).await;
        model.select(Some(a.clone()));
        view_until(&mut pane, |pane| version(pane) == Some(1)).await;
        model.set_viewport(OutlineViewport { generation, visible: 0..2, open: vec![(0, a.clone())] });

        let stream = opened.recv().await.unwrap();
        stream.hints.send(Ok(Change::Resync)).unwrap();
        settle().await;
        tokio::time::advance(GATHER).await;
        roots_in.notified().await;
        actor_in.notified().await;
        live.pause();
        settle().await;
        roots_out.notify_one();
        actor_out.notify_one();
        tokio::time::advance(FALLBACK * 2).await;
        settle().await;

        let view = list.borrow().clone();
        assert_eq!(view.rows.len(), 1, "the refresh's rows were dropped");
        assert!(view.footer().is_some_and(|footer| footer.contains("not refreshed")), "{:?}", view.footer());
        assert!(view.children[&a].not_refreshed(), "the chosen parent's read was dropped");
        assert_eq!(version(&pane.borrow()), Some(1), "the refresh's pane read was dropped");
        assert!(stream.hints.is_closed());
        assert_eq!(script.remaining(CHILDREN), 1, "no read after the pause");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_refresh_cancelled_after_its_root_rows_marks_only_the_parents_it_dropped() {
    with_clock(async {
        const CHILDREN: &str = "/actors/counter/a/children?limit=100";
        let (fresh_children, children_in, children_out) = later(page([json!("counter/y")], None));
        let script = Script::new([
            (ROOTS, vec![roots(&["a"]), roots(&["a", "b"])]),
            (SAMPLE, vec![page([], None), page([], None)]),
            (CHILDREN, vec![page([json!("counter/x")], None), fresh_children]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let (session, _nav, live, mut opened) = live_session(&server.url, &dir, Section::Actors).await;
        overview_read(&session).await;
        let model = session.actors();
        let mut list = model.list();
        model.show();
        let generation = view_until(&mut list, |list| list.rows.len() == 1).await.generation;
        let a = root("a");
        model.open(generation, &a);
        view_until(&mut list, |list| list.children.get(&a).is_some_and(|page| page.paging == Paging::Idle)).await;
        model.set_viewport(OutlineViewport { generation, visible: 0..2, open: vec![(0, a.clone())] });

        let stream = opened.recv().await.unwrap();
        stream.hints.send(Ok(Change::Resync)).unwrap();
        settle().await;
        tokio::time::advance(GATHER).await;
        children_in.notified().await;
        assert_eq!(list.borrow().rows.len(), 2, "the root rows landed");
        live.pause();
        settle().await;
        children_out.notify_one();
        settle().await;

        let view = list.borrow().clone();
        assert!(view.footer().is_none(), "the root rows were refreshed: {:?}", view.footer());
        assert!(view.children[&a].not_refreshed());
        assert_eq!(view.children[&a].rows, [ActorAddress::parse("counter/x").unwrap()]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn no_live_refresh_starts_on_a_page_already_left() {
    with_clock(async {
        let script = Script::new([(ROOTS, vec![roots(&["a"]), roots(&["a", "b"])]), (SAMPLE, vec![page([], None)])]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let (session, mut nav, _live, mut opened) = live_session(&server.url, &dir, Section::Actors).await;
        overview_read(&session).await;
        let model = session.actors();
        let mut list = model.list();
        model.show();
        view_until(&mut list, |list| list.rows.len() == 1).await;
        let stream = opened.recv().await.unwrap();

        // The user leaves Actors before the live model hears of it, and its batch falls due.
        session.navigate(&mut nav, |n| {
            n.select(Section::Nodes);
            true
        });
        stream.hints.send(Ok(Change::Resync)).unwrap();
        settle().await;
        tokio::time::advance(GATHER).await;
        settle().await;
        assert_eq!(script.remaining(ROOTS), 1, "no read of the Actors page");
        assert_eq!(list.borrow().rows.len(), 1);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_manual_reload_works_while_paused_and_arms_no_timer() {
    with_clock(async {
        let script = Script::new([
            (ROOTS, vec![roots(&["a"]), roots(&["a", "b"]), roots(&["a", "b", "c"])]),
            (SAMPLE, vec![page([], None), page([], None), page([], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let (session, _nav, live, mut opened) = live_session(&server.url, &dir, Section::Actors).await;
        overview_read(&session).await;
        let model = session.actors();
        let mut list = model.list();
        model.show();
        let before = view_until(&mut list, |list| list.rows.len() == 1).await;
        let stream = opened.recv().await.unwrap();
        live.pause();
        settle().await;
        assert!(stream.hints.is_closed());

        model.refresh();
        live.refreshed(Section::Actors);
        let reloaded = view_until(&mut list, |list| list.rows.len() == 2).await;
        assert!(reloaded.generation > before.generation, "⌘R reloads while paused");
        settle().await;
        tokio::time::advance(FALLBACK * 3).await;
        settle().await;
        assert_eq!(script.remaining(ROOTS), 1, "no automatic refresh after it");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn retry_of_a_child_page_marks_what_the_refresh_it_cancels_dropped() {
    with_clock(async {
        const CHILDREN_A: &str = "/actors/counter/a/children?limit=100";
        const CHILDREN_B: &str = "/actors/counter/b/children?limit=100";
        let (fresh_roots, roots_in, roots_out) = later(roots(&["a", "b", "c"]));
        let script = Script::new([
            (ROOTS, vec![roots(&["a", "b"]), fresh_roots]),
            (SAMPLE, vec![page([], None), page([], None)]),
            (CHILDREN_A, vec![page([json!("counter/x")], None)]),
            (CHILDREN_B, vec![problem(503), page([json!("counter/y")], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = crate::actors::Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let generation = view_until(&mut list, |list| list.rows.len() == 2).await.generation;
        let (a, b) = (root("a"), root("b"));
        model.open(generation, &a);
        model.open(generation, &b);
        view_until(&mut list, |list| {
            list.children.get(&a).is_some_and(|page| page.paging == Paging::Idle)
                && list.children.get(&b).is_some_and(|page| matches!(page.paging, Paging::Failed(_)))
        })
        .await;

        model.soft_refresh(OutlineViewport { generation, visible: 0..3, open: vec![(0, a.clone())] });
        roots_in.notified().await;
        assert!(model.retry_children(generation, &b), "Retry starts");
        roots_out.notify_one();
        let view = view_until(&mut list, |list| list.children[&b].rows.len() == 1).await;
        assert_eq!(view.rows.len(), 2, "the cancelled refresh's rows were dropped");
        assert!(view.footer().is_some_and(|footer| footer.contains("not refreshed")), "{:?}", view.footer());
        assert!(view.children[&a].not_refreshed(), "the chosen parent's read was dropped");
    })
    .await;
}
