//! Order the write, ordinary pages, soft refreshes and pane reads at the server.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::actors::{ListView, OutlineViewport, Paging};
use crate::live::{Live, LiveStatus, Pages};
use crate::model::{Navigation, Section};

const ROOTS: &str = "/actors/states?limit=100";
const DETAILS: &str = "/actors/counter/a";
const MAILBOX: &str = "/actors/counter/a/mailbox?limit=50";

fn paused(pane: &PaneView) -> bool {
    matches!(&pane.actor, Read::Ready(Some(actor)) if actor.paused)
}

fn row(key: &str, parent: Option<&str>) -> serde_json::Value {
    json!({
        "address": path(key).address(), "type": "counter", "state": {}, "version": 0,
        "createdAt": "2026-11-27T10:00:00.000Z", "paused": false, "parent": parent,
    })
}

fn page(items: impl serde::Serialize, next: Option<&str>) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"items": items, "next": next}))
}

fn later(answer: Fake) -> (Fake, Arc<Notify>, Arc<Notify>) {
    let Fake::Answer(status, body) = answer else { panic!("a response is held") };
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    (Fake::Later { arrived: arrived.clone(), release: release.clone(), status, body }, arrived, release)
}

async fn shown(model: &Actors) -> ListView {
    model.show();
    model.list().wait_for(|list| !list.rows.is_empty() && list.paging == Paging::Idle).await.unwrap().clone()
}

#[tokio::test(start_paused = true)]
async fn a_write_updates_the_list_and_pane_with_live_paused() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let session =
            Arc::new(Session::new(config(&dir, &[("local", &server.url)]), Handle::current(), Limits::DEFAULT));
        session.use_context(None);
        session.navigate(&mut Navigation::default(), |nav| {
            nav.select(Section::Actors);
            true
        });
        let pages: Arc<dyn Pages> = session.clone();
        let live = Live::new(&Handle::current(), Arc::downgrade(&pages), Duration::ZERO);
        live.pause();
        let (visit, context, client) = session.live_target();
        live.follow(context, client, Section::Actors, "", visit);
        live.status().wait_for(|status| *status == LiveStatus::Paused).await.unwrap();

        let model = session.actors();
        let list = shown(model).await;
        let pane = select(model, "a").await;
        assert_notice(model, &pane, Action::Pause, "paused").await;
        let changed = model.pane().wait_for(paused).await.unwrap().clone();
        assert_eq!(changed.generation, pane.generation);
        let refreshed = model.list().wait_for(|list| list.rows[0].paused).await.unwrap().clone();
        assert_eq!(refreshed.generation, list.generation);
        assert_eq!(*live.status().borrow(), LiveStatus::Paused);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_write_queues_a_new_pane_read_behind_the_running_soft_refresh() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let (old, arrived, release) = later(Fake::Answer(StatusCode::OK, row("a", None)));
        let reads = Arc::new(AtomicUsize::new(0));
        let count = reads.clone();
        let rule: Rule = Arc::new(move |target| {
            (target == DETAILS && count.fetch_add(1, Ordering::SeqCst) == 1).then(|| old.clone())
        });
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        shown(&model).await;
        select(&model, "a").await;
        let pane = model.pane().wait_for(pane_read).await.unwrap().clone();
        // Let the selection task release its slot before starting the ordinary refresh.
        while model.pane_reading() {
            tokio::task::yield_now().await;
        }
        let old_refresh = model.soft_refresh(OutlineViewport::default());
        arrived.notified().await;
        assert_notice(&model, &pane, Action::Pause, "paused").await;
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert!(!paused(&model.pane().borrow()));

        release.notify_one();
        old_refresh.await;
        model.pane().wait_for(paused).await.unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 3);
        assert_eq!(model.pane().borrow().generation, pane.generation);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn pausing_an_automatic_refresh_preserves_the_queued_and_running_write_refreshes() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let (old, old_in, old_out) = later(Fake::Answer(StatusCode::OK, row("a", None)));
        let (fresh, fresh_in, fresh_out) = {
            let mut data = row("a", None);
            data["paused"] = json!(true);
            later(Fake::Answer(StatusCode::OK, data))
        };
        let reads = AtomicUsize::new(0);
        let rule: Rule = Arc::new(move |target| match target {
            DETAILS => match reads.fetch_add(1, Ordering::SeqCst) {
                1 => Some(old.clone()),
                2 => Some(fresh.clone()),
                _ => None,
            },
            _ => None,
        });
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        shown(&model).await;
        select(&model, "a").await;
        let pane = model.pane().wait_for(pane_read).await.unwrap().clone();
        while model.pane_reading() {
            tokio::task::yield_now().await;
        }
        let automatic = model.soft_refresh(OutlineViewport::default());
        old_in.notified().await;
        assert_notice(&model, &pane, Action::Pause, "paused").await;
        model.cancel_soft_refresh();
        automatic.await;
        fresh_in.notified().await;
        model.cancel_soft_refresh();
        old_out.notify_one();
        fresh_out.notify_one();
        model.pane().wait_for(paused).await.unwrap();
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn blocked_paging_reads_the_pane_only_after_the_write_answer() {
    with_clock(async {
        for failed in [false, true] {
            let actors = actors().await;
            let actor = create(&actors, "a").await;
            actors.send(&actor, Work::Ok).await;
            let (slow_page, page_in, page_out) = later(page(Vec::<serde_json::Value>::new(), None));
            let (slow_mailbox, mailbox_in, mailbox_out) = later(page(Vec::<serde_json::Value>::new(), None));
            let (write_rule, write_in, write_out) = held("/actors/counter/a/mailbox/purge");
            let mailbox_reads = Arc::new(AtomicUsize::new(0));
            let reads = mailbox_reads.clone();
            let root_reads = Arc::new(AtomicUsize::new(0));
            let roots = root_reads.clone();
            let rule: Rule = Arc::new(move |target| match target {
                ROOTS if roots.fetch_add(1, Ordering::SeqCst) == 0 => Some(page([row("a", None)], Some("next"))),
                "/actors/states?cursor=next&limit=100" => {
                    Some(if failed { super::super::problem(503) } else { slow_page.clone() })
                }
                MAILBOX if reads.fetch_add(1, Ordering::SeqCst) == 0 => Some(slow_mailbox.clone()),
                _ => write_rule(target),
            });
            let server = serve(&actors, rule, Duration::ZERO).await;
            let model = model(&server);
            shown(&model).await;
            let selection = select(&model, "a").await;
            mailbox_in.notified().await;
            model.more();
            if failed {
                model.list().wait_for(|list| matches!(list.paging, Paging::Failed(_))).await.unwrap();
            } else {
                page_in.notified().await;
            }
            let sheet = model.press_action(&selection, Action::Purge).unwrap();
            model.answer_action(sheet.id, true);
            write_in.notified().await;
            assert_eq!(mailbox_reads.load(Ordering::SeqCst), 1);
            assert!(matches!(model.pane().borrow().mailbox, Read::Loading));
            write_out.notify_one();

            let outcome = model.actions().wait_for(|v| !v.busy).await.unwrap().feedback.clone();
            assert_eq!(outcome, Some(notice(selection.generation, "purged 1 messages")));
            model
                .pane()
                .wait_for(|pane| matches!(&pane.mailbox, Read::Ready(mailbox) if mailbox.rows.is_empty()))
                .await
                .unwrap();
            assert_eq!(mailbox_reads.load(Ordering::SeqCst), 2);
            assert_eq!(root_reads.load(Ordering::SeqCst), 1);
            mailbox_out.notify_one();
            if !failed {
                page_out.notify_one();
                model.list().wait_for(|list| !list.more && list.paging == Paging::Idle).await.unwrap();
            }
        }
    })
    .await;
}

struct Family;

impl Actor for Family {
    type Ref = Counter;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, _: Work) -> HandlerResult {
        if let Some(id) = match turn.self_ref().id.as_str() {
            "a" => Some("b"),
            "b" => Some("c"),
            _ => None,
        } {
            let child = Counter { id: id.into() };
            turn.create_child(&child, &json!({}))?;
            turn.send(&child, Work::Ok)?;
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn delete_prunes_every_loaded_occurrence_and_fences_ordinary_pages() {
    with_clock(async {
        for reselect in [false, true] {
            let actors = TestActors::builder().with(|node| node.host(Family)).build().await.unwrap();
            let parent = create(&actors, "a").await;
            create(&actors, "z").await;
            actors.send(&parent, Work::Ok).await;
            assert_eq!(actors.run_until_idle().await, 3);
            let (old_root, root_in, root_out) = later(page([row("b", Some("counter/a"))], None));
            let (old_child, child_in, child_out) = later(page(["counter/c"], None));
            let (new_root, new_root_in, new_root_out) = later(page(Vec::<serde_json::Value>::new(), None));
            let (new_child, new_child_in, new_child_out) = later(page(Vec::<serde_json::Value>::new(), None));
            let (delete, delete_in, delete_out) = held(DETAILS);
            let hold_delete = Arc::new(AtomicBool::new(false));
            let hold = hold_delete.clone();
            let roots = AtomicUsize::new(0);
            let children = AtomicUsize::new(0);
            let first_root = AtomicBool::new(true);
            let first_children = AtomicBool::new(true);
            let rule: Rule = Arc::new(move |target| match target {
                DETAILS if hold.swap(false, Ordering::SeqCst) => delete(target),
                ROOTS if first_root.swap(false, Ordering::SeqCst) => Some(page(
                    [row("a", None), row("b", Some("counter/a")), row("c", Some("counter/b")), row("z", None)],
                    Some("old-root"),
                )),
                "/actors/states?cursor=old-root&limit=100" => {
                    Some(if roots.fetch_add(1, Ordering::SeqCst) == 0 { old_root.clone() } else { new_root.clone() })
                }
                "/actors/counter/z/children?limit=100" if first_children.swap(false, Ordering::SeqCst) => {
                    Some(page(["counter/b"], Some("old-child")))
                }
                "/actors/counter/z/children?cursor=old-child&limit=100" => {
                    Some(if children.fetch_add(1, Ordering::SeqCst) == 0 {
                        old_child.clone()
                    } else {
                        new_child.clone()
                    })
                }
                _ => None,
            });
            let server = serve(&actors, rule, Duration::ZERO).await;
            let model = model(&server);
            let generation = shown(&model).await.generation;
            let mut list = model.list();
            let a = path("a");
            let b = path("b");
            let z = path("z");
            let ab = a.child(b.address().clone());
            model.open(generation, &a);
            model.open(generation, &b);
            model.open(generation, &z);
            list.wait_for(|list| {
                [&a, &b, &z].into_iter().all(|path| list.children.get(path).is_some_and(|page| !page.rows.is_empty()))
            })
            .await
            .unwrap();
            model.open(generation, &ab);
            list.wait_for(|list| list.children.get(&ab).is_some_and(|page| !page.rows.is_empty())).await.unwrap();
            model.more();
            root_in.notified().await;
            assert!(model.more_children(generation, &z));
            child_in.notified().await;

            let pane = select(&model, "a").await;
            model.pane().wait_for(pane_read).await.unwrap();
            hold_delete.store(true, Ordering::SeqCst);
            let sheet = model.press_action(&pane, Action::Delete).unwrap();
            model.answer_action(sheet.id, true);
            delete_in.notified().await;
            let other = if reselect {
                select(&model, "z").await;
                Some(model.pane().wait_for(pane_read).await.unwrap().clone())
            } else {
                None
            };
            delete_out.notify_one();
            let result = model.actions().wait_for(|v| !v.busy).await.unwrap().feedback.clone();
            let view = model.pane().borrow().clone();
            if let Some(other) = other {
                assert_eq!((view, result), (other, None));
            } else {
                assert!(view.not_found());
                assert_eq!(view.address.as_ref(), Some(a.address()));
                assert_eq!(result, Some(notice(view.generation, "deleted")));
            }
            let pruned = list.borrow().clone();
            assert_eq!(pruned.rows.iter().map(|row| &row.address).collect::<Vec<_>>(), [z.address()]);
            assert_eq!(pruned.children.keys().collect::<Vec<_>>(), [&z]);
            assert!(pruned.children[&z].rows.is_empty());
            assert_eq!(pruned.open.iter().collect::<Vec<_>>(), [&z]);
            assert_eq!(pruned.generation, generation);

            for key in ["a", "b", "c"] {
                assert!(server.client.get_actor(path(key).address()).await.unwrap().is_none());
            }
            // The earlier requests answer while their replacements still wait.
            new_root_in.notified().await;
            new_child_in.notified().await;
            root_out.notify_one();
            child_out.notify_one();
            for task in model.take_aborted() {
                let _ = task.await;
            }
            assert_eq!(list.borrow().rows.len(), 1);
            assert!(list.borrow().children[&z].rows.is_empty());
            new_root_out.notify_one();
            new_child_out.notify_one();
            list.wait_for(|list| !list.more && list.paging == Paging::Idle && list.children[&z].paging == Paging::Idle)
                .await
                .unwrap();
            assert_eq!(list.borrow().rows.len(), 1);
            assert!(list.borrow().children[&z].rows.is_empty());
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_post_write_refresh_replaces_a_pre_write_pane_read_when_paging_is_idle() {
    with_clock(async {
        let actors = actors().await;
        let actor = create(&actors, "a").await;
        actors.send(&actor, Work::Ok).await;
        let (old, old_in, old_out) = later(page(Vec::<serde_json::Value>::new(), None));
        let reads = Arc::new(AtomicUsize::new(0));
        let count = reads.clone();
        let rule: Rule = Arc::new(move |target| {
            (target == MAILBOX && count.fetch_add(1, Ordering::SeqCst) == 0).then(|| old.clone())
        });
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        shown(&model).await;
        let pane = select(&model, "a").await;
        old_in.notified().await;
        assert_notice(&model, &pane, Action::Purge, "purged 1 messages").await;
        model
            .pane()
            .wait_for(|pane| matches!(&pane.mailbox, Read::Ready(mailbox) if mailbox.rows.is_empty()))
            .await
            .unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        old_out.notify_one();
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_soft_refresh_started_before_delete_cannot_reinsert_the_actor() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        create(&actors, "z").await;
        let (old, old_in, old_out) = later(page([row("a", None), row("z", None)], None));
        let (fresh, fresh_in, fresh_out) = later(page([row("z", None)], None));
        let reads = AtomicUsize::new(0);
        let rule: Rule = Arc::new(move |target| match target {
            ROOTS => match reads.fetch_add(1, Ordering::SeqCst) {
                1 => Some(old.clone()),
                2 => Some(fresh.clone()),
                _ => None,
            },
            _ => None,
        });
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        shown(&model).await;
        let a = select(&model, "a").await;
        let prior = model.soft_refresh(OutlineViewport::default());
        old_in.notified().await;

        assert_notice(&model, &a, Action::Delete, "deleted").await;
        assert_eq!(model.list().borrow().rows.len(), 1);
        old_out.notify_one();
        prior.await;
        fresh_in.notified().await;
        assert_eq!(model.list().borrow().rows.len(), 1);
        assert!(model.pane().borrow().not_found());
        fresh_out.notify_one();
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_timed_out_write_alerts_and_reconciles_a_write_the_server_already_made() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let (ready, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let held = Fake::HoldResponse { ready: ready.clone(), release: release.clone() };
        let rule: Rule = Arc::new(move |target| (target == "/actors/counter/a/mailbox/pause").then(|| held.clone()));
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        let before = shown(&model).await;
        let selected = select(&model, "a").await;
        model.press_action(&selected, Action::Pause);
        ready.notified().await;
        assert!(model.actions().borrow().busy);
        assert!(!paused(&model.pane().borrow()));
        assert!(!model.list().borrow().rows[0].paused);

        tokio::time::advance(Limits::DEFAULT.deadline).await;
        let result = model.actions().wait_for(|view| !view.busy).await.unwrap().feedback.clone();
        let Some(Feedback::Alert(alert)) = result else { panic!("expected a timeout, got {result:?}") };
        assert_eq!(alert.title, crate::model::TIMED_OUT);
        let pane = model.pane().wait_for(paused).await.unwrap().clone();
        assert_eq!(pane.generation, selected.generation);
        let list = model.list().wait_for(|list| list.rows[0].paused).await.unwrap().clone();
        assert_eq!(list.generation, before.generation);
        release.notify_one();
    })
    .await;
}
