//! The Send Message sheet through the in-process HTTP server, on virtual time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::prelude::*;
use serde_json::json;
use tokio::runtime::Handle;

use super::actions::{create, model, select};
use super::soft_refresh::with_clock;
use super::types::registered;
use super::{Fake, Rule, Server, actors, no_rule, problem, serve};
use crate::actors::actions::{CHECK_DELAY, Choices, MAX_TEXT, SendView, Verdict};
use crate::actors::{ActorPath, Actors, Read};
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

/// A type no actor hosts, registered over HTTP, with one instance `form/a`. Its message types in pop-up order: `Any`
/// (`true`), `Draft7` (a draft-07 schema with a format and a draft 2020-12 keyword), `Outside` (a `$ref` to a server),
/// `Pattern` (a pattern the validator does not know) and `Required` (`n`, an integer).
async fn form(server: &Server) -> ActorAddress {
    let messages = [
        ("Any", json!(true)),
        (
            "Draft7",
            json!({"$schema": "http://json-schema.org/draft-07/schema#", "type": "object",
                "properties": {"mail": {"type": "string", "format": "email"}}, "dependentRequired": {"a": ["b"]}}),
        ),
        ("Outside", json!({"$ref": "http://127.0.0.1:9/outside.json"})),
        ("Pattern", json!({"type": "object", "properties": {"code": {"type": "string", "pattern": "["}}})),
        ("Required", json!({"type": "object", "required": ["n"], "properties": {"n": {"type": "integer"}}})),
    ];
    let messages = messages.into_iter().map(|(name, schema)| (name.to_owned(), schema)).collect();
    let contract = cicada::contract::Contract::new(cicada::TypeName::parse("form").unwrap(), json!(true), messages);
    server.client.register_type(&contract).await.unwrap();
    let address = ActorAddress::parse("form/a").unwrap();
    server.client.create_actor(&address, json!({})).await.unwrap();
    address
}

const ANY: usize = 0;
const DRAFT7: usize = 1;
const OUTSIDE: usize = 2;
const PATTERN: usize = 3;
const REQUIRED: usize = 4;

/// Opens the sheet on `form/a` once its message types are in.
async fn open_form(server: &Server) -> (Actors, u64) {
    form(server).await;
    let model = model(server);
    model.select(Some(ActorPath::root(ActorAddress::parse("form/a").unwrap())));
    let pane = model.pane().wait_for(|pane| matches!(pane.actor, Read::Ready(Some(_)))).await.unwrap().clone();
    let id = model.open_send(&pane, &no_types()).expect("the sheet opens");
    assert_eq!(choices(&model).await.note(), None);
    (model, id)
}

/// The local verdict on `text` as the message type at `index`, once the editor has been still.
async fn verdict(model: &Actors, id: u64, index: usize, text: &str) -> SendView {
    model.choose_send_type(id, index);
    model.edit_send_payload(id, text);
    assert_eq!(model.send_sheet().borrow().as_ref().unwrap().verdict, Verdict::Checking, "Send is off at once");
    tokio::time::advance(CHECK_DELAY).await;
    sheet_until(model, |view| view.verdict != Verdict::Checking && view.schema.is_some()).await
}

async fn wait_for_hold(arrival: std::sync::mpsc::Receiver<()>) {
    tokio::task::spawn_blocking(move || arrival.recv_timeout(Duration::from_secs(10)).unwrap()).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_local_verdict_agrees_with_the_server_on_a_type_no_actor_hosts() {
    with_clock(async {
        let actors = actors().await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        let address = ActorAddress::parse("form/a").unwrap();
        let cases = [
            (REQUIRED, json!({})),
            (REQUIRED, json!({"n": 1})),
            (REQUIRED, json!({"n": "x"})),
            (DRAFT7, json!({"mail": "nobody"})),
            (DRAFT7, json!({"a": 1})),
            (DRAFT7, json!({"a": 1, "b": 2})),
            (PATTERN, json!({"code": 5})),
            (OUTSIDE, json!({"x": 1})),
            (ANY, json!([])),
        ];
        let names = ["Any", "Draft7", "Outside", "Pattern", "Required"];
        for (index, payload) in cases {
            let view = verdict(&model, id, index, &payload.to_string()).await;
            let local = matches!(view.verdict, Verdict::Passes | Verdict::Unchecked);
            let answer =
                server.client.send_message(&address, names[index], payload.clone(), SendOptions::default()).await;
            let remote = match answer {
                Ok(receipt) => !receipt.duplicate,
                Err(error) if error.status() == Some(422) => false,
                Err(error) => panic!("{error}"),
            };
            assert_eq!(local, remote, "{} {payload}: {:?}", names[index], view.verdict);
            let unusable = index == PATTERN || index == OUTSIDE;
            assert_eq!(view.compiles, !unusable, "{}", names[index]);
            assert_eq!(view.schema_note().is_some(), unusable);
            if unusable {
                assert_eq!(view.verdict, Verdict::Unchecked);
            }
            if !local {
                assert_eq!(view.verdict, Verdict::Refused(Verdict::MISMATCH.into()));
            }
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn size_parse_and_depth_are_refused_before_the_schema() {
    with_clock(async {
        let actors = actors().await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        let nested = |levels: usize| format!("{}{}", "[".repeat(levels), "]".repeat(levels));
        assert_eq!(verdict(&model, id, ANY, &nested(64)).await.verdict, Verdict::Passes);
        let deep = verdict(&model, id, ANY, &nested(65)).await.verdict;
        assert_eq!(deep, Verdict::Refused("nested deeper than 64 levels".into()));
        let deep = verdict(&model, id, PATTERN, &nested(65)).await.verdict;
        assert_eq!(deep, Verdict::Refused("nested deeper than 64 levels".into()), "also when the schema is unusable");
        let large = format!("\"{}\"", "x".repeat(MAX_TEXT - 1));
        assert_eq!(verdict(&model, id, ANY, &large).await.verdict, Verdict::Refused("larger than 1 MiB".into()));
        let Verdict::Refused(why) = verdict(&model, id, ANY, "{").await.verdict else { panic!("refused") };
        assert!(why.starts_with("does not parse: "), "{why}");
        let Verdict::Refused(why) = verdict(&model, id, PATTERN, "{").await.verdict else { panic!("refused") };
        assert!(why.starts_with("does not parse: "), "{why}");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn one_check_runs_at_a_time_and_a_result_for_an_older_payload_is_dropped() {
    with_clock(async {
        let actors = actors().await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        assert_eq!(verdict(&model, id, REQUIRED, r#"{"n": 1}"#).await.verdict, Verdict::Passes);

        let (first, release_first) = model.hold_send_check();
        model.edit_send_payload(id, "{}");
        tokio::time::advance(CHECK_DELAY).await;
        wait_for_hold(first).await;
        let (second, release_second) = model.hold_send_check();
        model.edit_send_payload(id, r#"{"n": 2}"#);
        tokio::time::advance(CHECK_DELAY / 2).await;
        model.edit_send_payload(id, r#"{"n": 3}"#);
        tokio::time::advance(CHECK_DELAY).await;
        release_first.send(()).unwrap();
        wait_for_hold(second).await;
        assert_eq!(model.send_sheet().borrow().as_ref().unwrap().verdict, Verdict::Checking, "the mismatch is dropped");
        release_second.send(()).unwrap();
        assert_eq!(sheet_until(&model, |view| view.verdict != Verdict::Checking).await.verdict, Verdict::Passes);
    })
    .await;
}
