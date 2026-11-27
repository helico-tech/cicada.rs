//! The outline's child reads and fold state, over the Actors tests' in-process HTTP server.

use super::*;
use crate::actors::Children;
use std::collections::BTreeSet;

fn page(items: impl IntoIterator<Item = impl Serialize>, next: Option<&str>) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"items": items.into_iter().collect::<Vec<_>>(), "next": next}))
}

fn child(parent: &ActorPath, address: &str) -> ActorPath {
    parent.child(ActorAddress::parse(address).unwrap())
}

async fn children_until(
    list: &mut watch::Receiver<ListView>,
    parent: &ActorPath,
    done: impl Fn(&Children) -> bool,
) -> ListView {
    list_until(list, |list| list.children.get(parent).is_some_and(&done)).await
}

#[tokio::test]
async fn a_parents_pages_are_lazy_and_use_its_cursor_once() {
    let actors = test_actors().await;
    for id in ["a", "b"] {
        actors.create(&Counter { id: id.into() }, &json!({})).await;
    }
    let (first_arrived, first_release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let (more_arrived, more_release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let row = |n| format!("bag/{n:03}");
    let held = |arrived: &Arc<Notify>, release: &Arc<Notify>, range: std::ops::Range<usize>, next| Fake::Later {
        arrived: arrived.clone(),
        release: release.clone(),
        status: StatusCode::OK,
        body: json!({"items": range.map(row).collect::<Vec<_>>(), "next": next}),
    };
    let first = held(&first_arrived, &first_release, 0..100, Some("opaque+/=?"));
    let more = held(&more_arrived, &more_release, 100..150, None);
    let rule: Rule = Arc::new(move |target| match target {
        "/actors/counter/a/children?limit=100" => Some(first.clone()),
        "/actors/counter/a/children?cursor=opaque%2B%2F%3D%3F&limit=100" => Some(more.clone()),
        _ => None,
    });
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    let (a, b) = (root("counter/a"), root("counter/b"));
    model.show();
    let g = settled(&mut list).await.generation;
    assert!(events(&mut server).iter().all(|event| !event.contains("/children")));

    model.open(g, &a);
    timeout(WAIT, first_arrived.notified()).await.unwrap();
    model.close(g, &a);
    model.open(g, &a);
    // This parent can load while the first parent is still in flight.
    model.open(g, &b);
    let view = children_until(&mut list, &b, |c| c.paging == Paging::Idle).await;
    assert!(view.children[&b].rows.is_empty() && !view.children[&b].more && !view.children[&b].not_found);
    assert_eq!(view.children[&a].paging, Paging::Loading);

    first_release.notify_one();
    let view = children_until(&mut list, &a, |c| c.paging == Paging::Idle).await;
    assert_eq!(view.children[&a].rows.len(), 100);
    assert!(view.children[&a].more);
    model.close(g, &a);
    model.open(g, &a);
    assert_eq!(list.borrow().children[&a], view.children[&a]);

    assert!(model.more_children(g, &a));
    timeout(WAIT, more_arrived.notified()).await.unwrap();
    assert!(!model.more_children(g, &a));
    model.close(g, &a);
    more_release.notify_one();
    let view = children_until(&mut list, &a, |c| c.paging == Paging::Idle && c.rows.len() == 150).await;
    assert!(!view.children[&a].more);
    model.open(g, &a);
    model.more_children(g, &a);
    let addresses: Vec<_> = list.borrow().children[&a].rows.iter().map(ToString::to_string).collect();
    assert_eq!(addresses, (0..150).map(row).collect::<Vec<_>>());
    let events = events(&mut server);
    assert_eq!(events.iter().filter(|e| e.starts_with("arrived /actors/counter/a/children?")).count(), 2, "{events:?}");
    assert!(events.iter().all(|e| !e.starts_with("served /actors/bag/")), "{events:?}");
}

#[tokio::test]
async fn a_failed_child_page_retries_its_cursor_and_a_missing_parent_is_not_found() {
    let actors = test_actors().await;
    let a_ref = Counter { id: "a".into() };
    actors.create(&a_ref, &json!({})).await;
    actors.create(&Counter { id: "gone".into() }, &json!({})).await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let reads = attempts.clone();
    let rule: Rule = Arc::new(move |target| match target {
        "/actors/counter/a/children?limit=100" => Some(page(["bag/a"], Some("cursor"))),
        "/actors/counter/a/children?cursor=cursor&limit=100" => Some(if reads.fetch_add(1, Ordering::SeqCst) == 0 {
            Fake::Answer(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"type":"about:blank", "status":500, "title":"private-token", "detail":"private-token"}),
            )
        } else {
            page(["bag/b"], None)
        }),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    let (a, gone) = (root("counter/a"), root("counter/gone"));
    model.show();
    let g = settled(&mut list).await.generation;
    assert!(server.client.delete_actor(gone.address()).await.unwrap());
    model.open(g, &gone);
    let view = children_until(&mut list, &gone, |c| c.paging == Paging::Idle).await;
    assert!(view.children[&gone].not_found);

    model.open(g, &a);
    children_until(&mut list, &a, |c| c.paging == Paging::Idle).await;
    model.more_children(g, &a);
    let failed = children_until(&mut list, &a, |c| matches!(c.paging, Paging::Failed(_))).await;
    assert_eq!(failed.children[&a].paging, Paging::Failed("HTTP 500".into()));
    assert_eq!(failed.children[&a].rows, [root("bag/a").address().clone()]);
    assert!(failed.children[&a].more);
    model.more_children(g, &a);
    model.close(g, &a);
    model.open(g, &a);
    assert_eq!(list.borrow().children[&a], failed.children[&a]);
    model.retry_children(g, &a);
    let view = children_until(&mut list, &a, |c| c.paging == Paging::Idle).await;
    assert_eq!(view.children[&a].rows.len(), 2);
    assert!(!view.children[&a].more);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn refresh_drops_an_in_flight_child_page_and_rereads_the_open_parent() {
    let actors = test_actors().await;
    actors.create(&Counter { id: "a".into() }, &json!({})).await;
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held = Fake::Later {
        arrived: arrived.clone(),
        release: release.clone(),
        status: StatusCode::OK,
        body: json!({"items": ["bag/stale"], "next": null}),
    };
    let reads = AtomicUsize::new(0);
    let rule: Rule = Arc::new(move |target| {
        (target == "/actors/counter/a/children?limit=100")
            .then(|| if reads.fetch_add(1, Ordering::SeqCst) == 0 { held.clone() } else { page(["bag/fresh"], None) })
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    let a = root("counter/a");
    model.show();
    let initial = settled(&mut list).await.generation;
    model.open(initial, &a);
    timeout(WAIT, arrived.notified()).await.unwrap();
    model.refresh();
    model.close(initial, &a);
    let view = children_until(&mut list, &a, |c| c.paging == Paging::Idle).await;
    assert!(view.generation > initial && view.open.contains(&a));
    release.notify_one();
    let aborted = model.take_aborted();
    assert_eq!(aborted.len(), 1);
    assert!(timeout(WAIT, aborted.into_iter().next().unwrap()).await.unwrap().unwrap_err().is_cancelled());
    assert_eq!(list.borrow().children[&a].rows, [root("bag/fresh").address().clone()]);
}

#[tokio::test]
async fn the_root_and_child_occurrences_select_and_open_independently() {
    let actors = test_actors().await;
    let bag = Bag { id: "a".into() };
    actors.create(&bag, &Contents { n: 7, name: "selected".into() }).await;
    actors.create(&Counter { id: "a".into() }, &json!({})).await;
    actors.send(&bag, Work::Ok).await;
    claim_and_abandon(actors.store(), &actors.system().hosted_types(), Duration::from_secs(600), 1).await.unwrap();
    let rule = watches_rule(|target| match target {
        "/actors/counter/a/children?limit=100" => Some(page(["bag/a", "counter/idle"], None)),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let (mut list, mut pane) = (model.list(), model.pane());
    let (parent, bag_root) = (root("counter/a"), root("bag/a"));
    let bag_child = child(&parent, "bag/a");
    model.show();
    let g = settled(&mut list).await.generation;
    model.open(g, &parent);
    let view = children_until(&mut list, &parent, |c| c.paging == Paging::Idle).await;
    assert_eq!(view.child_status(bag_child.address()), "running");
    assert_eq!(view.child_status(&view.children[&parent].rows[1]), "");
    model.open(g, &bag_root);
    assert!(list.borrow().open.contains(&bag_root));
    assert!(!list.borrow().open.contains(&bag_child));
    model.select(Some(bag_root.clone()));
    let root_details = answered(&mut pane).await;
    model.select(Some(bag_child.clone()));
    let child_details = answered(&mut pane).await;
    assert_eq!(model.selected(), Some(bag_child.clone()));
    assert_eq!(root_details.address, child_details.address);
    assert_eq!(root_details.actor, child_details.actor);
    assert!(matches!(child_details.actor, Read::Ready(Some(_))));
    model.close(g, &bag_root);
    model.open(g, &bag_child);
    assert_eq!(list.borrow().open, BTreeSet::from([parent.clone(), bag_child.clone()]));

    model.search("counter/");
    assert_eq!(model.selected(), Some(bag_child));
    let view = settled(&mut list).await;
    assert!(view.open.is_empty());
    model.open(view.generation, &parent);
    let view = children_until(&mut list, &parent, |c| c.paging == Paging::Idle).await;
    assert_eq!(view.child_status(bag_root.address()), "—");

    model.use_client(Some(server.client.clone()));
    model.show();
    let view = settled(&mut list).await;
    assert!(view.open.is_empty() && view.children.is_empty() && model.selected().is_none());
}

#[tokio::test]
async fn reload_keeps_later_root_folds_until_reaching_their_address() {
    let actors = test_actors().await;
    for n in 0..105 {
        actors.create(&Counter { id: format!("{n:03}") }, &json!({})).await;
    }
    let server = serve(&actors, no_rule(), Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    model.show();
    let view = settled(&mut list).await;
    let g = view.generation;
    let (passed, survives, ended) = (root("counter/010"), root("counter/102"), root("counter/104"));
    model.more();
    settled(&mut list).await;
    for path in [&passed, &survives, &ended] {
        model.open(g, path);
        children_until(&mut list, path, |c| c.paging == Paging::Idle).await;
    }
    for path in [&passed, &ended] {
        assert!(server.client.delete_actor(path.address()).await.unwrap());
    }

    // These edits occur in a single poll, so only the original prefix is read after the debounce.
    model.search("b");
    model.search("");
    let first = settled(&mut list).await;
    assert!(first.generation > g && first.more);
    assert_eq!(first.open, BTreeSet::from([survives.clone(), ended]));
    assert!(first.children.is_empty(), "the surviving open row is on a later page");
    assert!(!first.wants_more(first.root_index(root("counter/020").address()).unwrap()));
    assert!(first.wants_more(first.root_index(root("counter/099").address()).unwrap()));
    model.more();
    let view = children_until(&mut list, &survives, |c| c.paging == Paging::Idle).await;
    assert_eq!(view.open, BTreeSet::from([survives]));
}

#[tokio::test]
async fn a_closed_parents_saved_folds_are_pruned_when_its_children_are_read_again() {
    let actors = test_actors().await;
    actors.create(&Counter { id: "a".into() }, &json!({})).await;
    let refreshed = Arc::new(AtomicBool::new(false));
    let after = refreshed.clone();
    let rule: Rule = Arc::new(move |target| match target {
        "/actors/counter/a/children?limit=100" => Some(if after.load(Ordering::SeqCst) {
            page(["bag/b"], Some("later"))
        } else {
            page(["bag/a", "bag/c", "bag/z"], None)
        }),
        "/actors/counter/a/children?cursor=later&limit=100" => Some(page(["bag/c"], None)),
        "/actors/bag/a/children?limit=100"
        | "/actors/bag/c/children?limit=100"
        | "/actors/bag/z/children?limit=100" => Some(page([] as [&str; 0], None)),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    let parent = root("counter/a");
    let (passed, survives, ended) = (child(&parent, "bag/a"), child(&parent, "bag/c"), child(&parent, "bag/z"));
    model.show();
    let g = settled(&mut list).await.generation;
    model.open(g, &parent);
    children_until(&mut list, &parent, |c| c.paging == Paging::Idle).await;
    for path in [&passed, &survives, &ended] {
        model.open(g, path);
        children_until(&mut list, path, |c| c.paging == Paging::Idle).await;
    }
    model.close(g, &parent);
    refreshed.store(true, Ordering::SeqCst);
    model.refresh();
    let view = settled(&mut list).await;
    assert_eq!(view.open, BTreeSet::from([passed, survives.clone(), ended.clone()]));
    assert!(view.children.is_empty());
    model.open(view.generation, &parent);
    let first = children_until(&mut list, &parent, |c| c.paging == Paging::Idle).await;
    assert_eq!(first.open, BTreeSet::from([parent.clone(), survives.clone(), ended]));
    assert!(!first.children.contains_key(&survives));
    model.more_children(first.generation, &parent);
    let view = children_until(&mut list, &survives, |c| c.paging == Paging::Idle).await;
    assert_eq!(view.open, BTreeSet::from([parent, survives]));
}
