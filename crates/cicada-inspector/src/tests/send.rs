//! The Send Message sheet through the in-process HTTP server, on virtual time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;
use tokio::runtime::Handle;

use super::actions::{create, model, select};
use super::soft_refresh::with_clock;
use super::types::registered;
use super::{Fake, Rule, Server, actors, no_rule, problem, serve};
use crate::actors::Actors;
use crate::actors::actions::{Choices, SendView};
use crate::model::Limits;
use crate::pager::Paging;
use crate::types::Types;

/// A Types model whose list is not read.
fn no_types() -> Types {
    Types::new(Handle::current(), Limits::DEFAULT)
}

async fn sheet_until(model: &Actors, done: impl FnMut(&SendView) -> bool) -> SendView {
    let mut done = done;
    model.send_sheet().wait_for(|view| view.as_ref().is_some_and(&mut done)).await.unwrap().clone().unwrap()
}

async fn choices(model: &Actors) -> Choices {
    sheet_until(model, |view| view.choices != Choices::Loading).await.choices
}

/// Opens the sheet on `counter/a`, selected in a fresh model.
async fn open(server: &Server, types: &Types) -> (Actors, u64) {
    let model = model(server);
    let pane = select(&model, "a").await;
    let id = model.open_send(&pane, types).expect("the sheet opens");
    (model, id)
}

fn served_types(server: &mut Server) -> usize {
    std::iter::from_fn(|| server.events.try_recv().ok()).filter(|event| event == "served /types").count()
}

#[tokio::test(start_paused = true)]
async fn the_pop_up_lists_the_message_types_from_one_read_of_types() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
        let (model, id) = open(&server, &no_types()).await;
        assert!(model.actions().borrow().busy);
        assert_eq!(choices(&model).await, Choices::Ready(vec!["Fail".into(), "Ok".into()]));
        let view = model.send_sheet().borrow().clone().unwrap();
        assert_eq!((view.id, view.address.as_str(), view.type_name.as_str()), (id, "counter/a", "counter"));
        assert_eq!(served_types(&mut server), 1);
        model.cancel_send(id);
        assert!(model.send_sheet().borrow().is_none());
        assert!(!model.actions().borrow().busy);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_pop_up_uses_the_types_page_list_when_it_holds_the_type() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
        let types = no_types();
        types.use_client(Some(server.client.clone()));
        types.show();
        types.list().wait_for(|list| list.paging == Paging::Idle && !list.rows.is_empty()).await.unwrap();
        assert_eq!(served_types(&mut server), 1);
        let (model, _) = open(&server, &types).await;
        assert_eq!(
            model.send_sheet().borrow().as_ref().unwrap().choices,
            Choices::Ready(vec!["Fail".into(), "Ok".into()])
        );
        assert_eq!(served_types(&mut server), 0);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_failed_read_of_types_shows_its_error_and_retry_reads_again() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let rule: Rule = Arc::new(move |target| {
            (target == "/types" && counted.fetch_add(1, Ordering::SeqCst) == 0).then(|| problem(500))
        });
        let server = serve(&actors, rule, Duration::ZERO).await;
        let (model, id) = open(&server, &no_types()).await;
        let Choices::Failed(error) = choices(&model).await else { panic!("the read failed") };
        assert!(error.contains("500"), "{error}");
        assert!(Choices::Failed(error).note().unwrap().starts_with("The message types could not be read: "));
        model.retry_send_types(id);
        assert_eq!(model.send_sheet().borrow().as_ref().unwrap().choices, Choices::Loading);
        assert_eq!(choices(&model).await, Choices::Ready(vec!["Fail".into(), "Ok".into()]));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_type_missing_from_types_or_without_messages_offers_no_send() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        for (items, expected, note) in [
            (vec![registered("bag")], Choices::NotRegistered, "type not registered"),
            (
                vec![json!({"name": "counter", "hash": "sha256:00", "state": true, "messages": {}})],
                Choices::NoMessages,
                "this type takes no messages",
            ),
        ] {
            let body = json!({ "items": items });
            let rule: Rule =
                Arc::new(move |target| (target == "/types").then(|| Fake::Answer(StatusCode::OK, body.clone())));
            let server = serve(&actors, rule, Duration::ZERO).await;
            let (model, _) = open(&server, &no_types()).await;
            let choices = choices(&model).await;
            assert_eq!(choices.note().as_deref(), Some(note));
            assert_eq!(choices, expected);
        }
    })
    .await;
}
