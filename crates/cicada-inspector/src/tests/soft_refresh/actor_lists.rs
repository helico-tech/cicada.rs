use super::*;
use crate::actors::{ActorPath, Actors, ListView, OutlineViewport, PaneView, Read, Running};
use tokio::sync::watch;

const ROOTS: &str = "/actors/states?limit=100";

fn roots(keys: &[&str], next: Option<&str>) -> Fake {
    page(
        keys.iter().map(|key| {
            json!({
                "address": format!("counter/{key}"), "type": "counter", "version": 1,
                "createdAt": "2026-11-27T10:00:00.000Z", "paused": false, "state": {}
            })
        }),
        next,
    )
}

fn actor_path(key: &str) -> ActorPath {
    ActorPath::root(ActorAddress::parse(&format!("counter/{key}")).unwrap())
}

fn children(keys: &[&str], next: Option<&str>) -> Fake {
    page(keys.iter().map(|key| json!(format!("counter/{key}"))), next)
}

fn viewport(generation: u64, visible: std::ops::Range<usize>, open: Vec<(usize, ActorPath)>) -> OutlineViewport {
    OutlineViewport { generation, visible, open }
}

async fn open(model: &Actors, list: &mut watch::Receiver<ListView>, generation: u64, path: &ActorPath) {
    model.open(generation, path);
    list.wait_for(|list| list.children.get(path).is_some_and(|page| page.paging == Paging::Idle)).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn nested_reads_wait_for_the_parent_and_skip_a_pruned_path() {
    with_clock(async {
        let (parent_refresh, arrived, release) = later(children(&["keep"], None));
        let script = Script::new([
            (ROOTS, vec![roots(&["a"], None), roots(&["a"], None)]),
            ("/actors/counter/a/children?limit=100", vec![children(&["gone", "keep"], None), parent_refresh]),
            ("/actors/counter/gone/children?limit=100", vec![children(&["grandchild"], None)]),
            ("/actors/counter/keep/children?limit=100", vec![children(&[], None), children(&["new"], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let generation = list.wait_for(|list| list.rows.len() == 1).await.unwrap().generation;
        let root = actor_path("a");
        let gone = root.child(actor_path("gone").address().clone());
        let keep = root.child(actor_path("keep").address().clone());
        for path in [&root, &gone, &keep] {
            open(&model, &mut list, generation, path).await;
        }
        model.select(Some(gone.child(actor_path("grandchild").address().clone())));
        model.soft_refresh(viewport(generation, 1..3, vec![(0, root.clone()), (1, gone.clone()), (2, keep.clone())]));
        arrived.notified().await;
        assert_eq!(script.remaining("/actors/counter/keep/children?limit=100"), 1);
        release.notify_one();
        let view = list.wait_for(|list| list.children[&keep].rows.len() == 1).await.unwrap();
        assert_eq!(view.open, std::collections::BTreeSet::from([root, keep]));
        assert!(!view.children.contains_key(&gone));
        drop(view);
        assert!(model.selected().is_none() && model.pane().borrow().address.is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn soft_marks_closed_and_retained_children_until_their_ranges_are_read() {
    with_clock(async {
        let script = Script::new([
            (ROOTS, vec![roots(&["a", "b"], None), roots(&["a", "b"], None), roots(&["a", "b"], None)]),
            (
                "/actors/counter/a/children?limit=100",
                vec![children(&["a", "d"], Some("old")), children(&["b"], Some("new"))],
            ),
            ("/actors/counter/a/children?cursor=old&limit=100", vec![children(&["e", "g"], None)]),
            ("/actors/counter/a/children?cursor=new&limit=100", vec![children(&["e", "f"], None)]),
            ("/actors/counter/b/children?limit=100", vec![children(&["x"], None), children(&["y"], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let generation = list.wait_for(|list| list.rows.len() == 2).await.unwrap().generation;
        let (a, b) = (actor_path("a"), actor_path("b"));
        open(&model, &mut list, generation, &a).await;
        open(&model, &mut list, generation, &b).await;
        assert!(model.more_children(generation, &a));
        list.wait_for(|list| !list.children[&a].more).await.unwrap();
        model.close(generation, &b);

        model.soft_refresh(viewport(generation, 0..1, vec![(0, a.clone())]));
        let view = list.wait_for(|list| list.children[&a].rows[0].as_str() == "counter/b").await.unwrap();
        assert!(view.children[&a].not_refreshed() && view.children[&b].not_refreshed());
        assert_eq!(view.children[&a].rows.len(), 4);
        drop(view);
        // More uses the refresh cursor, covers the kept range once and removes its missing rows.
        model.more_children(generation, &a);
        let view = list.wait_for(|list| !list.children[&a].more).await.unwrap();
        assert_eq!(
            view.children[&a].rows.iter().map(ActorAddress::as_str).collect::<Vec<_>>(),
            ["counter/b", "counter/e", "counter/f"]
        );
        assert!(!view.children[&a].not_refreshed());
        drop(view);
        open(&model, &mut list, generation, &b).await;
        assert!(list.borrow().children[&b].not_refreshed());

        model.soft_refresh(viewport(generation, 1..2, vec![(1, b.clone())]));
        let view = list.wait_for(|list| !list.children[&b].not_refreshed()).await.unwrap();
        assert_eq!(view.children[&b].rows[0].as_str(), "counter/y");
        assert!(view.children[&a].not_refreshed());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn roots_end_by_address_even_when_the_old_last_row_is_gone() {
    with_clock(async {
        let script = Script::new([
            (ROOTS, vec![roots(&["b", "d"], Some("old")), roots(&["a", "b"], Some("first"))]),
            ("/actors/states?cursor=first&limit=100", vec![roots(&["c", "e"], Some("second"))]),
            ("/actors/states?cursor=second&limit=100", vec![roots(&["f"], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let generation = list.wait_for(|list| list.paging == Paging::Idle && list.more).await.unwrap().generation;
        model.soft_refresh(OutlineViewport::default());
        let view = list.wait_for(|list| list.rows.len() == 4).await.unwrap();
        let keys = view.rows.iter().map(|row| row.address.as_str()).collect::<Vec<_>>();
        assert_eq!(keys, ["counter/a", "counter/b", "counter/c", "counter/e"]);
        assert!(view.more);
        assert_eq!(view.generation, generation);
        assert_eq!(script.remaining("/actors/states?cursor=second&limit=100"), 1);
        drop(view);
        model.more();
        assert!(!list.wait_for(|list| list.rows.len() == 5).await.unwrap().more);
    })
    .await;
}

const SAMPLE: &str = "/turns?limit=200";
const CHILDREN_A: &str = "/actors/counter/a/children?limit=100";

#[tokio::test(start_paused = true)]
async fn soft_refresh_waits_for_root_and_first_child_reads_and_remembers_one_request() {
    with_clock(async {
        let (more, root_in, root_done) = later(roots(&["b"], None));
        let (first, child_in, child_done) = later(children(&["old"], None));
        let script = Script::new([
            (ROOTS, vec![roots(&["a"], Some("old")), roots(&["a", "b"], None)]),
            ("/actors/states?cursor=old&limit=100", vec![more]),
            (CHILDREN_A, vec![first, children(&["new"], None)]),
            (SAMPLE, vec![page([], None), page([], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let generation =
            list.wait_for(|list| list.rows.len() == 1 && list.running != Running::Loading).await.unwrap().generation;
        let a = actor_path("a");
        model.open(generation, &a);
        child_in.notified().await;
        model.more();
        root_in.notified().await;
        for _ in 0..3 {
            model.soft_refresh(viewport(generation, 0..1, vec![(0, a.clone())]));
        }
        root_done.notify_one();
        list.wait_for(|list| list.rows.len() == 2).await.unwrap();
        assert_eq!(script.remaining(ROOTS), 1);
        assert_eq!(script.remaining(SAMPLE), 1);
        child_done.notify_one();
        list.wait_for(|list| list.children[&a].rows.first().is_some_and(|row| row.as_str() == "counter/new"))
            .await
            .unwrap();
        assert_eq!(script.remaining(ROOTS), 0);
        assert_eq!(script.remaining(SAMPLE), 0);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn more_waits_for_the_refresh_and_uses_both_new_cursors() {
    with_clock(async {
        let (refresh, root_in, root_done) = later(roots(&["a"], Some("new")));
        let (child_refresh, child_in, child_done) = later(children(&["fresh"], Some("new")));
        let script = Script::new([
            (ROOTS, vec![roots(&["a"], Some("old")), refresh]),
            ("/actors/states?cursor=new&limit=100", vec![roots(&["b"], None)]),
            (CHILDREN_A, vec![children(&["old"], Some("old")), child_refresh]),
            ("/actors/counter/a/children?cursor=new&limit=100", vec![children(&["later"], None)]),
            (SAMPLE, vec![page([], None), page([], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let generation = list.wait_for(|list| list.rows.len() == 1).await.unwrap().generation;
        let a = actor_path("a");
        open(&model, &mut list, generation, &a).await;

        model.soft_refresh(viewport(generation, 0..1, vec![(0, a.clone())]));
        root_in.notified().await;
        model.more();
        assert!(model.more_children(generation, &a));
        root_done.notify_one();
        child_in.notified().await;
        assert_eq!(script.remaining("/actors/states?cursor=new&limit=100"), 1);
        assert_eq!(script.remaining("/actors/counter/a/children?cursor=new&limit=100"), 1);
        child_done.notify_one();
        let view = list.wait_for(|list| list.rows.len() == 2 && !list.children[&a].more).await.unwrap();
        assert_eq!(
            view.children[&a].rows.iter().map(ActorAddress::as_str).collect::<Vec<_>>(),
            ["counter/fresh", "counter/later"]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn retry_of_a_failed_parent_cancels_the_refresh_and_runs_at_its_held_cursor() {
    with_clock(async {
        let (refresh, started, release) = later(roots(&["a", "b", "obsolete"], None));
        let script = Script::new([
            (ROOTS, vec![roots(&["a", "b"], None), refresh]),
            (CHILDREN_A, vec![children(&["old"], Some("old"))]),
            ("/actors/counter/a/children?cursor=old&limit=100", vec![problem(503), children(&["retry"], None)]),
            ("/actors/counter/b/children?limit=100", vec![children(&[], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        let generation = list.wait_for(|list| list.rows.len() == 2).await.unwrap().generation;
        let (a, b) = (actor_path("a"), actor_path("b"));
        open(&model, &mut list, generation, &a).await;
        open(&model, &mut list, generation, &b).await;
        assert!(model.more_children(generation, &a));
        list.wait_for(|list| matches!(list.children[&a].paging, Paging::Failed(_))).await.unwrap();
        model.soft_refresh(viewport(generation, 0..2, vec![(0, a.clone()), (1, b)]));
        started.notified().await;
        assert_eq!(list.borrow().children[&a].paging, Paging::Failed("HTTP 503".into()));
        assert!(model.retry_children(generation, &a));
        let view = list.wait_for(|list| !list.children[&a].more).await.unwrap();
        assert!(view.children[&a].rows.contains(actor_path("retry").address()));
        drop(view);
        release.notify_one();
        assert_eq!(list.borrow().rows.len(), 2);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn failed_root_paging_discards_a_remembered_refresh() {
    with_clock(async {
        let (failed, started, release) = later(problem(503));
        let script = Script::new([
            (ROOTS, vec![roots(&["a"], Some("old")), roots(&["obsolete"], None)]),
            ("/actors/states?cursor=old&limit=100", vec![failed, roots(&["b"], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        list.wait_for(|list| list.rows.len() == 1).await.unwrap();
        model.more();
        started.notified().await;
        model.soft_refresh(OutlineViewport::default());
        release.notify_one();
        list.wait_for(|list| matches!(list.paging, Paging::Failed(_))).await.unwrap();
        model.retry();
        list.wait_for(|list| list.rows.len() == 2).await.unwrap();
        assert_eq!(script.remaining(ROOTS), 1);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_late_initial_sample_never_overwrites_the_soft_sample() {
    with_clock(async {
        let (initial, started, release) = later(page([turns::turn("counter/a", "old")], None));
        let script = Script::new([
            (ROOTS, vec![roots(&["a", "b"], None), roots(&["a", "b"], None)]),
            (SAMPLE, vec![initial, page([turns::turn("counter/b", "new")], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        started.notified().await;
        list.wait_for(|list| list.rows.len() == 2).await.unwrap();
        model.soft_refresh(OutlineViewport::default());
        list.wait_for(|list| list.child_status(actor_path("b").address()) == "running").await.unwrap();
        let initial = model.take_sample().unwrap();
        release.notify_one();
        initial.await.unwrap();
        assert_eq!(list.borrow().child_status(actor_path("a").address()), "");
        assert_eq!(list.borrow().child_status(actor_path("b").address()), "running");
    })
    .await;
}

const DETAILS: &str = "/actors/counter/a";
const TURN: &str = "/actors/counter/a/turn";
const MAILBOX: &str = "/actors/counter/a/mailbox?limit=50";
const WATCHING: &str = "/watches?limit=50&watcher=counter%2Fa";
const WATCHED_BY: &str = "/watches?limit=50&target=counter%2Fa";

fn details(version: u64) -> Fake {
    Fake::Answer(
        StatusCode::OK,
        json!({
            "address": "counter/a", "type": "counter", "createdAt": "2026-11-27T10:00:00.000Z",
            "paused": false, "version": version, "state": {"version": version}
        }),
    )
}

async fn pane_answered(pane: &mut watch::Receiver<PaneView>) -> PaneView {
    pane.wait_for(|p| {
        !matches!(p.actor, Read::Loading)
            && !matches!(p.turn, Read::Loading)
            && !matches!(p.mailbox, Read::Loading)
            && !matches!(p.watching, Read::Loading)
            && !matches!(p.watched_by, Read::Loading)
    })
    .await
    .unwrap()
    .clone()
}

#[tokio::test(start_paused = true)]
async fn actor_refresh_shares_four_slots_and_one_deadline_and_marks_failed_children() {
    with_clock(async {
        let (root, root_in, root_done) = later(roots(&["a", "b"], None));
        let (sample, sample_in, _) = later(page([], None));
        let (actor, actor_in, _) = later(details(2));
        let (turn, turn_in, _) = later(Fake::Answer(StatusCode::OK, turns::turn("counter/a", "new")));
        let (child, child_in, _) = later(children(&["replaced"], None));
        let script = Script::new([
            (ROOTS, vec![roots(&["a", "b"], None), root]),
            (SAMPLE, vec![page([turns::turn("counter/a", "old")], None), sample]),
            (CHILDREN_A, vec![children(&["old"], None), child]),
            ("/actors/counter/b/children?limit=100", vec![children(&["kept"], None), problem(503)]),
            (DETAILS, vec![details(1), actor]),
            (TURN, vec![Fake::Answer(StatusCode::OK, turns::turn("counter/a", "old")), turn]),
            (MAILBOX, vec![page([], None), page([], None)]),
            (WATCHING, vec![page([], None), page([], None)]),
            (WATCHED_BY, vec![page([], None), page([], None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits { deadline: Duration::from_secs(20), ..Limits::DEFAULT });
        model.use_client(Some(server.client));
        let (mut list, mut pane) = (model.list(), model.pane());
        model.show();
        let generation =
            list.wait_for(|list| list.rows.len() == 2 && list.running != Running::Loading).await.unwrap().generation;
        let (a, b) = (actor_path("a"), actor_path("b"));
        open(&model, &mut list, generation, &a).await;
        open(&model, &mut list, generation, &b).await;
        model.select(Some(a.clone()));
        pane_answered(&mut pane).await;

        let start = Instant::now();
        model.soft_refresh(viewport(generation, 1..2, vec![(0, a.clone()), (1, b.clone())]));
        root_in.notified().await;
        sample_in.notified().await;
        actor_in.notified().await;
        turn_in.notified().await;
        for route in [MAILBOX, WATCHING, WATCHED_BY, CHILDREN_A] {
            assert_eq!(script.remaining(route), 1, "{route} waits for a slot");
        }
        root_done.notify_one();
        child_in.notified().await;
        for route in [MAILBOX, WATCHING, WATCHED_BY] {
            assert_eq!(script.remaining(route), 0, "{route} used the freed slot");
        }
        let failed = list.wait_for(|list| list.children[&b].refresh_error.is_some()).await.unwrap();
        assert_eq!(failed.children[&b].refresh_error.as_deref(), Some("HTTP 503"));
        assert_eq!(failed.children[&b].rows, [actor_path("kept").address().clone()]);
        assert!(failed.children[&b].not_refreshed());
        drop(failed);
        tokio::time::advance(Duration::from_secs(9)).await;
        assert!(list.borrow().children[&a].refresh_error.is_none());
        tokio::time::advance(Duration::from_secs(1)).await;
        let view = list.wait_for(|list| list.children[&a].refresh_error.is_some()).await.unwrap();
        assert_eq!(Instant::now(), start + Duration::from_secs(10));
        assert_eq!(view.children[&a].refresh_error.as_deref(), Some(TIMED_OUT));
        assert_eq!(view.children[&a].rows, [actor_path("old").address().clone()]);
        assert!(view.children[&a].not_refreshed());
        drop(view);
        let view = list.wait_for(|list| list.footer().is_some_and(|footer| footer.contains(TIMED_OUT))).await.unwrap();
        assert_eq!(view.child_status(a.address()), "running");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn root_refresh_caps_full_states_at_three_pages_and_keeps_the_sorted_tail() {
    with_clock(async {
        let numbered = |range: std::ops::Range<usize>, next| {
            let keys: Vec<_> = range.map(|n| format!("{n:03}")).collect();
            roots(&keys.iter().map(String::as_str).collect::<Vec<_>>(), next)
        };
        let script = Script::new([
            (ROOTS, vec![numbered(50..150, Some("old1")), numbered(0..100, Some("new1"))]),
            ("/actors/states?cursor=old1&limit=100", vec![numbered(150..250, Some("old2"))]),
            ("/actors/states?cursor=old2&limit=100", vec![numbered(250..350, Some("old3"))]),
            ("/actors/states?cursor=new1&limit=100", vec![numbered(100..200, Some("new2"))]),
            ("/actors/states?cursor=new2&limit=100", vec![numbered(200..300, Some("new3"))]),
            ("/actors/states?cursor=new3&limit=100", vec![numbered(300..349, None)]),
        ]);
        let server = serve(&actors().await, script.rule(), Duration::ZERO).await;
        let model = Actors::new(Handle::current(), Limits::DEFAULT);
        model.use_client(Some(server.client));
        let mut list = model.list();
        model.show();
        for size in [100, 200] {
            list.wait_for(|list| list.rows.len() == size).await.unwrap();
            model.more();
        }
        list.wait_for(|list| list.rows.len() == 300).await.unwrap();
        model.soft_refresh(OutlineViewport::default());
        let view = list.wait_for(|list| list.rows.len() == 350).await.unwrap();
        for (index, row) in view.rows.iter().enumerate() {
            assert_eq!(row.address.as_str(), format!("counter/{index:03}"));
            assert_eq!(view.root_index(&row.address), Some(index));
        }
        assert_eq!(script.remaining("/actors/states?cursor=new3&limit=100"), 1);
        drop(view);
        model.more();
        let view = list.wait_for(|list| !list.more).await.unwrap();
        assert_eq!(view.rows.len(), 349);
        assert_eq!(view.rows.last().unwrap().address.as_str(), "counter/348");
    })
    .await;
}
