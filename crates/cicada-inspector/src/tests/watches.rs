//! Watches reads against an in-process server, including local filtering and Show Actor history.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::ActorAddress;
use serde_json::{Value, json};
use tokio::runtime::Handle;

use super::{Counter, Fake, Rule, Server, actors, config, problem, serve, view_until};
use crate::actors::ActorPath;
use crate::model::{Limits, Navigation, Section, Session};
use crate::pager::Paging;
use crate::watches::Watches;

pub(super) fn row(target: &str, watcher: &str) -> Value {
    json!({"target": target, "watcher": watcher})
}

fn page(items: Vec<Value>, next: Option<&str>) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"items": items, "next": next}))
}

fn served(server: &mut Server) -> Vec<String> {
    std::iter::from_fn(|| server.events.try_recv().ok()).filter(|event| event.starts_with("served /watches")).collect()
}

#[tokio::test]
async fn watches_filter_pauses_scroll_and_load_more_and_retry_use_the_exact_cursor() {
    let actors = actors().await;
    let fail = Arc::new(AtomicBool::new(true));
    let failing = fail.clone();
    let rule: Rule = Arc::new(move |path| match path {
        "/watches?limit=100" => {
            Some(page((0..100).map(|n| row(&format!("bag/{n:03}"), "observer/a")).collect(), Some("one/+ =")))
        }
        "/watches?cursor=one%2F%2B+%3D&limit=100" if failing.load(Ordering::SeqCst) => Some(problem(503)),
        "/watches?cursor=one%2F%2B+%3D&limit=100" => Some(page(vec![row("counter/a", "observer/b")], Some("two"))),
        "/watches?cursor=two&limit=100" => Some(page(vec![row("counter/b", "observer/c")], Some("three"))),
        "/watches?cursor=three&limit=100" => Some(page(vec![row("counter/c", "observer/c")], None)),
        _ => None,
    });
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = Watches::new(Handle::current(), Limits::DEFAULT);
    model.use_client(Some(server.client.clone()));
    let mut list = model.list();
    model.show();
    let first = view_until(&mut list, |list| list.loaded == 100).await;
    assert_eq!(first.rows.len(), 100);
    assert!(first.more && !first.wants_more(79) && first.wants_more(80));
    assert_eq!(served(&mut server), ["served /watches?limit=100"]);
    model.scroll(79);
    assert_eq!(list.borrow().paging, Paging::Idle);

    model.search("absent");
    assert!(list.borrow().rows.is_empty() && list.borrow().more && !list.borrow().wants_more(99));
    model.scroll(99);
    model.scroll(0);
    assert_eq!(list.borrow().paging, Paging::Idle);
    assert!(served(&mut server).is_empty());

    model.more();
    let failed = view_until(&mut list, |list| matches!(list.paging, Paging::Failed(_))).await;
    assert!(failed.rows.is_empty() && failed.more);
    assert!(matches!(failed.paging, Paging::Failed(error) if error.contains("503")));
    assert_eq!(served(&mut server), ["served /watches?cursor=one%2F%2B+%3D&limit=100"]);
    fail.store(false, Ordering::SeqCst);
    model.retry();
    let loaded = view_until(&mut list, |list| list.loaded == 101).await;
    assert!(loaded.rows.is_empty() && loaded.more);
    model.scroll(100);
    assert_eq!(list.borrow().paging, Paging::Idle);
    assert_eq!(served(&mut server), ["served /watches?cursor=one%2F%2B+%3D&limit=100"]);

    model.more();
    let manual = view_until(&mut list, |list| list.loaded == 102).await;
    assert!(manual.rows.is_empty() && manual.more);
    model.scroll(101);
    assert_eq!(list.borrow().paging, Paging::Idle);
    assert_eq!(served(&mut server), ["served /watches?cursor=two&limit=100"]);

    model.search("counter/");
    let by_target = list.borrow().rows[0].clone();
    model.search("observer/b");
    assert_eq!(list.borrow().rows.len(), 1);
    assert!(Arc::ptr_eq(&by_target, &list.borrow().rows[0]));
    model.select(Some(&by_target));
    assert_eq!(*model.pane().borrow(), Some(by_target));
    model.search("");
    assert!(list.borrow().wants_more(101));
    model.scroll(101);
    let last = view_until(&mut list, |list| list.loaded == 103).await;
    assert_eq!(last.rows.last().unwrap().target.as_str(), "counter/c");
    assert!(!last.more);
    assert_eq!(served(&mut server), ["served /watches?cursor=three&limit=100"]);

    model.refresh();
    assert!(model.pane().borrow().is_none());
    view_until(&mut list, |list| list.loaded == 100).await;
    assert_eq!(served(&mut server), ["served /watches?limit=100"]);
    model.select(Some(&first.rows[0]));
    model.search("observer/");
    let rule: Rule =
        Arc::new(|path| (path == "/watches?limit=100").then(|| page(vec![row("bag/other", "observer/other")], None)));
    let mut other = serve(&actors, rule, Duration::ZERO).await;
    model.use_client(Some(other.client.clone()));
    assert!(list.borrow().rows.is_empty() && model.pane().borrow().is_none());
    assert!(served(&mut other).is_empty());
    model.show();
    model.show();
    let fresh = view_until(&mut list, |list| list.loaded == 1).await;
    assert_eq!(fresh.prefix, "observer/");
    assert_eq!(fresh.rows[0].target.as_str(), "bag/other");
    assert_eq!(served(&mut other), ["served /watches?limit=100"]);
}

#[tokio::test]
async fn watches_show_actor_selects_the_watcher_or_one_actor_target_and_replays_the_jump() {
    let actors = actors().await;
    for id in ["a", "aa", "b", "bb"] {
        actors.create(&Counter { id: id.to_owned() }, &json!({})).await;
    }
    let fail = Arc::new(AtomicBool::new(true));
    let failing = fail.clone();
    let rule: Rule = Arc::new(move |path| match path {
        "/watches?limit=100" => Some(page(
            vec![
                row("*", "counter/a"),
                row("counter", "counter/a"),
                row("counter/b", "counter/a"),
                row("counter/gone", "counter/a"),
            ],
            None,
        )),
        "/actors/states?prefix=counter%2Fa&limit=100" if failing.swap(false, Ordering::SeqCst) => Some(problem(503)),
        _ => None,
    });
    let server = serve(&actors, rule, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(config(&dir, &[("prod", &server.url)]), Handle::current(), Limits::DEFAULT);
    session.use_context(None);
    let watches = session.watches();
    let (mut list, pane) = (watches.list(), watches.pane());
    let mut nav = Navigation::default();
    nav.select(Section::Watches);
    watches.show();
    let loaded = view_until(&mut list, |list| list.loaded == 4).await;
    for watch in &loaded.rows[..2] {
        watches.select(Some(watch));
        assert!(watches.actor(false).is_some());
        assert!(watches.actor(true).is_none());
    }

    let (mut actor_list, mut actor_pane) = (session.actors().list(), session.actors().pane());
    watches.search("counter/b");
    let watch = &loaded.rows[2];
    watches.select(Some(watch));
    let watcher = watches.actor(false).unwrap();
    session.show_actor(&mut nav, watcher.clone());
    view_until(&mut actor_list, |list| matches!(list.paging, Paging::Failed(_))).await;
    assert!(session.actors().selected().is_none() && !actor_pane.borrow().not_found());
    session.actors().retry();
    view_until(&mut actor_pane, |pane| pane.address.as_ref() == Some(&watcher)).await;
    assert_eq!(session.actors().selected(), Some(ActorPath::root(watcher)));
    assert!(session.navigate(&mut nav, Navigation::back));
    assert_eq!(nav.current(), Section::Watches);
    assert_eq!(list.borrow().prefix, "counter/b");
    assert_eq!(pane.borrow().as_ref(), Some(watch));

    let target = watches.actor(true).unwrap();
    session.show_actor(&mut nav, target.clone());
    view_until(&mut actor_pane, |pane| pane.address.as_ref() == Some(&target)).await;
    assert_eq!(session.actors().selected(), Some(ActorPath::root(target.clone())));
    assert!(session.navigate(&mut nav, Navigation::back));
    assert_eq!(pane.borrow().as_ref(), Some(watch));
    assert!(session.navigate(&mut nav, Navigation::forward));
    assert_eq!(nav.current(), Section::Actors);
    view_until(&mut actor_pane, |pane| pane.address.as_ref() == Some(&target)).await;
    assert_eq!(session.actors().selected(), Some(ActorPath::root(target)));

    assert!(session.navigate(&mut nav, Navigation::back));
    watches.search("counter/gone");
    watches.select(Some(&loaded.rows[3]));
    session.show_actor(&mut nav, watches.actor(true).unwrap());
    let absent = view_until(&mut actor_pane, |pane| pane.not_found()).await;
    assert_eq!(absent.address, Some(ActorAddress::parse("counter/gone").unwrap()));
    assert!(session.actors().selected().is_none());
    session.stop();
}
