//! Types reads against an in-process server, and formatting ordered by a test barrier.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::{Value, json};
use tokio::runtime::Handle;

use super::{Fake, Rule, actors, problem, serve, view_until};
use crate::json::CUT;
use crate::model::Limits;
use crate::pager::Paging;
use crate::types::Types;

fn registered(name: &str) -> Value {
    json!({
        "name": name, "hash": "sha256:0123456789abcdef",
        "state": {"title": format!("{name} state")},
        "messages": {"Zebra": {"title": "zebra", "description": "x".repeat(CUT)}, "Alpha": true}
    })
}

fn types_page(names: &[&str]) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"items": names.iter().map(|name| registered(name)).collect::<Vec<_>>()}))
}

#[tokio::test]
async fn types_reads_once_per_visit_reload_or_context_and_filters_the_cache() {
    let actors = actors().await;
    let rule: Rule = Arc::new(|path| (path == "/types").then(|| types_page(&["bag", "counter"])));
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = Types::new(Handle::current(), Limits::DEFAULT);
    let mut list = model.list();
    let pane = model.pane();
    model.use_client(Some(server.client.clone()));
    assert_eq!(*list.borrow().rows, []);
    assert!(server.events.try_recv().is_err());

    model.show();
    model.show();
    let first = view_until(&mut list, |list| list.rows.len() == 2).await;
    assert_eq!(first.rows[0].name, "bag");
    assert_eq!(first.rows[0].hash, "sha256:0123456789abcdef");
    assert_eq!(first.rows[0].messages.len(), 2);
    assert_eq!(server.events.try_recv().unwrap(), "served /types");
    model.search("cou");
    assert!(Arc::ptr_eq(&first.rows[1], &list.borrow().rows[0]));
    model.show();
    assert!(server.events.try_recv().is_err());

    model.select(Some("counter"));
    model.choose(2);
    assert_eq!(pane.borrow().schema, 2);
    model.refresh();
    assert!(pane.borrow().row.is_none());
    assert_eq!(pane.borrow().schema, 0);
    view_until(&mut list, |list| list.paging == Paging::Idle && list.rows.len() == 1).await;
    assert_eq!(server.events.try_recv().unwrap(), "served /types");

    let rule: Rule = Arc::new(|path| (path == "/types").then(|| types_page(&["coupons"])));
    let mut other = serve(&actors, rule, Duration::ZERO).await;
    model.select(Some("counter"));
    model.use_client(Some(other.client.clone()));
    assert!(list.borrow().rows.is_empty() && pane.borrow().row.is_none());
    assert!(other.events.try_recv().is_err());
    model.show();
    view_until(&mut list, |list| list.index("coupons").is_some()).await;
    assert_eq!(list.borrow().prefix, "cou");
    assert_eq!(other.events.try_recv().unwrap(), "served /types");
    assert!(server.events.try_recv().is_err() && other.events.try_recv().is_err());
    model.search("Counter");
    assert!(list.borrow().rows.is_empty());
}

#[tokio::test]
async fn types_failed_read_discards_old_types_and_retry_reads_again() {
    let actors = actors().await;
    let fail = Arc::new(AtomicBool::new(false));
    let fail_request = fail.clone();
    let rule: Rule = Arc::new(move |path| {
        (path == "/types")
            .then(|| if fail_request.load(Ordering::SeqCst) { problem(503) } else { types_page(&["bag"]) })
    });
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = Types::new(Handle::current(), Limits::DEFAULT);
    model.use_client(Some(server.client.clone()));
    let mut list = model.list();
    model.show();
    view_until(&mut list, |list| list.rows.len() == 1).await;
    fail.store(true, Ordering::SeqCst);
    model.refresh();
    let failed = view_until(&mut list, |list| matches!(list.paging, Paging::Failed(_))).await;
    assert!(failed.rows.is_empty());
    assert!(matches!(failed.paging, Paging::Failed(error) if !error.is_empty()));
    model.search("ba");
    assert!(list.borrow().rows.is_empty());
    fail.store(false, Ordering::SeqCst);
    model.retry();
    view_until(&mut list, |list| list.rows.len() == 1).await;
    assert_eq!(list.borrow().prefix, "ba");
    assert_eq!(std::iter::from_fn(|| server.events.try_recv().ok()).collect::<Vec<_>>(), ["served /types"; 3]);
}

#[tokio::test]
async fn types_formats_only_the_visible_choice_and_drops_a_superseded_format() {
    let actors = actors().await;
    let rule: Rule = Arc::new(|path| (path == "/types").then(|| types_page(&["bag", "counter"])));
    let server = serve(&actors, rule, Duration::ZERO).await;
    let model = Types::new(Handle::current(), Limits::DEFAULT);
    model.use_client(Some(server.client));
    model.show();
    view_until(&mut model.list(), |list| list.rows.len() == 2).await;
    let mut pane = model.pane();
    assert_eq!(model.formatted(), 0);
    model.select(Some("bag"));
    assert_eq!(pane.borrow().schemas(), ["State", "Alpha", "Zebra"]);
    model.choose(2);
    assert_eq!(model.formatted(), 0);
    model.set_pane_visible(true);
    let shown = view_until(&mut pane, |pane| pane.json.is_some()).await;
    let json = shown.json.unwrap();
    assert_eq!(shown.schema, 2);
    assert!(json.cut && json.text.len() <= CUT);
    assert_eq!(model.formatted(), 1);

    let (arrived, release) = model.hold_format();
    model.choose(0);
    tokio::task::spawn_blocking(move || arrived.recv_timeout(Duration::from_secs(10))).await.unwrap().unwrap();
    model.choose(2);
    model.select(Some("counter"));
    model.choose(1);
    assert!(pane.borrow().json.is_none());
    assert_eq!(model.formatted(), 1);
    release.send(()).unwrap();
    let shown = view_until(&mut pane, |pane| pane.json.is_some()).await;
    assert_eq!(shown.row.unwrap().name, "counter");
    assert_eq!(shown.json.unwrap().text, "true");
    assert_eq!(model.formatted(), 3, "only the held schema and the last choice ran");
    model.choose(0);
    let state = view_until(&mut pane, |pane| pane.json.is_some()).await;
    assert!(state.json.unwrap().text.contains("counter state"));
    model.search("bag");
    assert!(pane.borrow().row.is_none());
}
