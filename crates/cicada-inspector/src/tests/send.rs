//! The Send Message sheet through the in-process HTTP server, on virtual time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use cicada::prelude::*;
use serde_json::json;
use tokio::{runtime::Handle, sync::Notify};

use super::actions::{create, model, notice, select};
use super::soft_refresh::with_clock;
use super::types::registered;
use super::{Fake, Rule, Server, actors, no_rule, problem, serve};
use crate::actors::actions::{Action, Alert, CHECK_DELAY, Choices, Feedback, MAX_TEXT, SendView, Verdict};
use crate::actors::{ActorPath, Actors, Read};
use crate::model::{Limits, TIMED_OUT};
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

const FORM_SEND: &str = "/actors/form/a/messages";

/// Types `text` as a `Required` payload and waits for it to pass.
async fn passing(model: &Actors, id: u64, text: &str) -> SendView {
    let view = verdict(model, id, REQUIRED, text).await;
    assert_eq!(view.verdict, Verdict::Passes, "{text}");
    assert!(view.can_send());
    view
}

/// Presses Send and waits for its answer: the sheet's view, or `None` once it closed.
async fn send(model: &Actors, id: u64) -> Option<SendView> {
    model.send_message(id);
    assert!(model.send_sheet().borrow().as_ref().is_some_and(|view| view.sending && !view.can_send()));
    model.send_sheet().wait_for(|view| view.as_ref().is_none_or(|view| !view.sending)).await.unwrap().clone()
}

fn feedback(model: &Actors) -> Option<Feedback> {
    model.actions().borrow().feedback.clone()
}

/// Holds the response to the first send to `form/a`, after the server handled it; later sends go through.
fn first_send_held() -> (Rule, Arc<Notify>, Arc<Notify>) {
    let (ready, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let held = Fake::HoldResponse { ready: ready.clone(), release: release.clone() };
    let calls = Arc::new(AtomicUsize::new(0));
    let rule: Rule = Arc::new(move |target| {
        (target == FORM_SEND && calls.fetch_add(1, Ordering::SeqCst) == 0).then(|| held.clone())
    });
    (rule, ready, release)
}

#[tokio::test(start_paused = true)]
async fn a_sent_message_closes_the_sheet_with_its_id_and_reads_the_pane_again() {
    with_clock(async {
        let actors = actors().await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        let view = passing(&model, id, r#"{"n": 1}"#).await;
        let generation = model.pane().borrow().generation;
        assert_eq!(send(&model, id).await, None);
        assert_eq!(feedback(&model), Some(notice(generation, &format!("sent {}", view.message_id))));
        assert!(!model.actions().borrow().busy);
        let pane = model
            .pane()
            .wait_for(|pane| matches!(&pane.mailbox, Read::Ready(mailbox) if !mailbox.rows.is_empty()))
            .await
            .unwrap()
            .clone();
        let Read::Ready(mailbox) = &pane.mailbox else { unreachable!() };
        assert_eq!((&mailbox.rows[0].id, mailbox.rows[0].message_type.as_str()), (&view.message_id, "Required"));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn send_after_a_timeout_reuses_the_id_and_shows_already_sent() {
    with_clock(async {
        let actors = actors().await;
        let (rule, ready, release) = first_send_held();
        let server = serve(&actors, rule, Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        let first = passing(&model, id, r#"{"n": 1}"#).await;
        model.send_message(id);
        ready.notified().await;
        tokio::time::advance(Limits::DEFAULT.deadline).await;
        let view = sheet_until(&model, |view| !view.sending).await;
        assert_eq!(view.refusal.as_ref().unwrap().title, TIMED_OUT);
        assert_eq!(view.message_id, first.message_id);
        assert!(view.can_send(), "the payload is kept and Send is on");
        release.notify_one();

        model.edit_send_payload(id, r#"{ "n" : 1 }"#);
        tokio::time::advance(CHECK_DELAY).await;
        let same = sheet_until(&model, |view| view.verdict == Verdict::Passes).await;
        assert_eq!(same.message_id, first.message_id, "an equal parsed payload keeps the id");
        let generation = model.pane().borrow().generation;
        assert_eq!(send(&model, id).await, None);
        assert_eq!(feedback(&model), Some(notice(generation, &format!("already sent {}", first.message_id))));
        let mailbox = server.client.mailbox(&ActorAddress::parse("form/a").unwrap(), None, None).await.unwrap();
        assert_eq!(mailbox.unwrap().items.len(), 1, "delivered once");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_edited_payload_after_a_lost_answer_goes_with_a_new_id() {
    with_clock(async {
        let actors = actors().await;
        let (rule, ready, release) = first_send_held();
        let server = serve(&actors, rule, Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        let first = passing(&model, id, r#"{"n": 1}"#).await;
        model.send_message(id);
        ready.notified().await;
        tokio::time::advance(Limits::DEFAULT.deadline).await;
        sheet_until(&model, |view| !view.sending).await;
        release.notify_one();

        let edited = passing(&model, id, r#"{"n": 2}"#).await;
        assert_ne!(edited.message_id, first.message_id);
        assert_eq!(send(&model, id).await, None);
        let generation = model.pane().borrow().generation;
        assert_eq!(feedback(&model), Some(notice(generation, &format!("sent {}", edited.message_id))));
        let mailbox = server.client.mailbox(&ActorAddress::parse("form/a").unwrap(), None, None).await.unwrap();
        let ids: Vec<_> = mailbox.unwrap().items.into_iter().map(|item| item.message.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&first.message_id) && ids.contains(&edited.message_id));
        // Another message type with the same payload is another message too.
        let (model, id) = open_form_again(&server).await;
        let required = passing(&model, id, r#"{"n": 1}"#).await;
        let any = verdict(&model, id, ANY, r#"{"n": 1}"#).await;
        assert_eq!(any.verdict, Verdict::Passes);
        assert_ne!(any.message_id, required.message_id);
    })
    .await;
}

/// Opens a second sheet on `form/a`, registered already.
async fn open_form_again(server: &Server) -> (Actors, u64) {
    let model = model(server);
    model.select(Some(ActorPath::root(ActorAddress::parse("form/a").unwrap())));
    let pane = model.pane().wait_for(|pane| matches!(pane.actor, Read::Ready(Some(_)))).await.unwrap().clone();
    let id = model.open_send(&pane, &no_types()).expect("the sheet opens");
    choices(&model).await;
    (model, id)
}

#[tokio::test(start_paused = true)]
async fn refusals_show_in_the_sheet_or_as_an_alert_and_keep_the_sheet_and_payload() {
    with_clock(async {
        let actors = actors().await;
        let answers = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from([
            Fake::Answer(
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({"type": "urn:cicada:problem:invalid-payload", "status": 422,
                    "title": "Invalid\u{0007} payload",
                    "detail": "Payload of 'Required' does not match the registered schema; GET /types/form has it"}),
            ),
            Fake::Answer(
                StatusCode::BAD_REQUEST,
                json!({"type": "urn:cicada:problem:bad-request", "status": 400, "title": "Bad request",
                    "detail": "The payload contains a NUL character"}),
            ),
            Fake::Answer(
                StatusCode::PAYLOAD_TOO_LARGE,
                json!({"type": "urn:cicada:problem:payload-too-large", "status": 413, "title": "Too large"}),
            ),
        ])));
        let rule: Rule =
            Arc::new(move |target| (target == FORM_SEND).then(|| answers.lock().unwrap().pop_front()).flatten());
        let server = serve(&actors, rule, Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        let view = passing(&model, id, r#"{"n": 1}"#).await;

        let refused = send(&model, id).await.expect("the sheet stays open");
        let refusal = refused.refusal.clone().unwrap();
        assert_eq!(refusal.title, "Invalid payload");
        assert_eq!(
            refusal.detail.as_deref(),
            Some("Payload of 'Required' does not match the registered schema; GET /types/form has it")
        );
        assert_eq!(
            (refused.alert.as_ref(), &refused.verdict, &refused.message_id),
            (None, &Verdict::Passes, &view.message_id)
        );
        assert!(refused.can_send() && model.actions().borrow().busy);

        let refused = send(&model, id).await.expect("the sheet stays open");
        assert_eq!(refused.refusal.unwrap().detail.as_deref(), Some("The payload contains a NUL character"));

        let refused = send(&model, id).await.expect("the sheet stays open");
        assert_eq!((&refused.refusal, refused.alert.as_ref().unwrap().title.as_str()), (&None, "Too large"));
        assert!(refused.can_send() && model.actions().borrow().busy);
        assert_eq!(feedback(&model), None);

        server.client.delete_actor(&ActorAddress::parse("form/a").unwrap()).await.unwrap();
        let refused = send(&model, id).await.expect("the sheet stays open");
        assert_eq!(refused.refusal.unwrap(), Alert { title: "actor not found".into(), detail: None });
        model.edit_send_payload(id, r#"{"n": 1}"#);
        assert_eq!(model.send_sheet().borrow().as_ref().unwrap().refusal, None, "an edit clears the refusal");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_sheet_and_the_actions_take_turns_and_a_context_switch_or_close_drops_the_send() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let model = model(&server);
        let pane = select(&model, "a").await;
        let sheet = model.press_action(&pane, Action::Purge).unwrap();
        assert_eq!(model.open_send(&pane, &no_types()), None, "a confirmation holds the slot");
        model.answer_action(sheet.id, false);
        let id = model.open_send(&pane, &no_types()).unwrap();
        assert!(model.press_action(&pane, Action::Pause).is_none(), "the sheet holds the slot");
        assert_eq!(model.open_send(&pane, &no_types()), None);
        model.cancel_send(id);
        assert!(!model.actions().borrow().busy && model.send_sheet().borrow().is_none());

        let ready = Arc::new(Notify::new());
        let held = Fake::HoldResponse { ready: ready.clone(), release: Arc::new(Notify::new()) };
        let rule: Rule = Arc::new(move |target| (target == FORM_SEND).then(|| held.clone()));
        let server = serve(&actors, rule, Duration::ZERO).await;
        form(&server).await;
        for close in [false, true] {
            let (model, id) = open_form_again(&server).await;
            passing(&model, id, r#"{"n": 1}"#).await;
            model.send_message(id);
            ready.notified().await;
            if close {
                model.stop();
            } else {
                model.use_client(Some(server.client.clone()));
            }
            for task in model.take_aborted() {
                assert!(task.await.is_err_and(|e| e.is_cancelled()));
            }
            assert!(model.send_sheet().borrow().is_none());
            assert_eq!(*model.actions().borrow(), Default::default());
        }
    })
    .await;
}

/// Code review 1, finding 1: a sheet reopened during a slow check waits for it on the same thread, and the closed
/// sheet's queued check is skipped.
#[tokio::test(start_paused = true)]
async fn sheets_share_one_check_thread_and_a_closed_sheets_queued_check_is_skipped() {
    with_clock(async {
        let actors = actors().await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        passing(&model, id, r#"{"n": 1}"#).await;
        let (slow, release_slow) = model.hold_send_check();
        model.edit_send_payload(id, r#"{"n": 2}"#);
        tokio::time::advance(CHECK_DELAY).await;
        wait_for_hold(slow).await;
        let ran = model.send_checks_run();
        model.edit_send_payload(id, r#"{"n": 3}"#);
        tokio::time::advance(CHECK_DELAY).await;
        model.cancel_send(id);

        let (next, release_next) = model.hold_send_check();
        let pane = model.pane().borrow().clone();
        let id = model.open_send(&pane, &no_types()).expect("the sheet opens again");
        choices(&model).await;
        let not_yet = tokio::task::spawn_blocking(move || {
            let early = next.recv_timeout(Duration::from_millis(200)).is_ok();
            (early, next)
        });
        let (early, next) = not_yet.await.unwrap();
        assert!(!early, "the new sheet's check waits for the running one");
        release_slow.send(()).unwrap();
        wait_for_hold(next).await;
        assert_eq!(model.send_checks_run(), ran + 1, "the closed sheet's queued check was skipped");
        release_next.send(()).unwrap();
        let view = sheet_until(&model, |view| view.verdict != Verdict::Checking && view.schema.is_some()).await;
        assert_eq!((view.id, view.chosen, view.verdict), (id, ANY, Verdict::Passes), "the new sheet's `{{}}` as Any");
    })
    .await;
}

/// Code review 1, findings 2 and 6: the draft and the type are frozen while a send is out, and Cancel then closes the
/// sheet but holds the slot until the send ends, dropping its answer.
#[tokio::test(start_paused = true)]
async fn the_draft_is_frozen_while_sending_and_cancel_holds_the_slot_until_the_send_ends() {
    with_clock(async {
        let actors = actors().await;
        let (rule, ready, release) = first_send_held();
        let server = serve(&actors, rule, Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        let before = passing(&model, id, r#"{"n": 1}"#).await;
        model.send_message(id);
        ready.notified().await;
        model.edit_send_payload(id, r#"{"n": 2}"#);
        model.choose_send_type(id, ANY);
        let frozen = model.send_sheet().borrow().clone().unwrap();
        assert!(frozen.sending);
        assert_eq!(
            (frozen.chosen, &frozen.verdict, &frozen.message_id),
            (REQUIRED, &Verdict::Passes, &before.message_id)
        );

        model.cancel_send(id);
        assert!(model.send_sheet().borrow().is_none(), "the sheet closes");
        assert!(model.actions().borrow().busy, "the slot is held while the send is out");
        let pane = model.pane().borrow().clone();
        assert!(model.press_action(&pane, Action::Purge).is_none());
        assert_eq!(model.open_send(&pane, &no_types()), None);
        release.notify_one();
        let view = model.actions().wait_for(|view| !view.busy).await.unwrap().clone();
        assert_eq!(view.feedback, None, "the answer is dropped");
        assert!(model.send_sheet().borrow().is_none());
        let mailbox = server.client.mailbox(&ActorAddress::parse("form/a").unwrap(), None, None).await.unwrap();
        assert_eq!(mailbox.unwrap().items[0].message.id, before.message_id, "the frozen draft was sent");
    })
    .await;
}

/// Code review 1, finding 6, with a timeout: the slot is freed when the send times out.
#[tokio::test(start_paused = true)]
async fn a_cancelled_send_that_times_out_frees_the_slot() {
    with_clock(async {
        let actors = actors().await;
        let (rule, ready, _release) = first_send_held();
        let server = serve(&actors, rule, Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        passing(&model, id, r#"{"n": 1}"#).await;
        model.send_message(id);
        ready.notified().await;
        model.cancel_send(id);
        assert!(model.actions().borrow().busy);
        tokio::time::advance(Limits::DEFAULT.deadline).await;
        let view = model.actions().wait_for(|view| !view.busy).await.unwrap().clone();
        assert_eq!(view.feedback, None);
    })
    .await;
}

/// Code review 1, finding 3: Send pressed for `counter/a`, then `counter/b` selected before the send is dispatched,
/// sends nothing.
#[tokio::test(start_paused = true)]
async fn a_selection_changed_before_dispatch_sends_nothing() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        create(&actors, "b").await;
        let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
        let (model, id) = open(&server, &no_types()).await;
        choices(&model).await;
        model.edit_send_payload(id, r#"{}"#);
        tokio::time::advance(CHECK_DELAY).await;
        let view = sheet_until(&model, |view| view.verdict != Verdict::Checking).await;
        assert!(view.can_send(), "{:?}", view.verdict);
        model.send_message(id);
        model.select(Some(ActorPath::root(ActorAddress::parse("counter/b").unwrap())));
        let generation = model.pane().borrow().generation;
        let view = model.actions().wait_for(|view| !view.busy).await.unwrap().clone();
        assert_eq!(view.feedback, Some(notice(generation, "selection changed; nothing done")));
        assert!(model.send_sheet().borrow().is_none());
        while let Ok(event) = server.events.try_recv() {
            assert!(!event.contains("/messages"), "{event}");
        }
    })
    .await;
}

/// Code review 1, finding 4: after a lost answer, a round trip through another message type gives a fresh id.
#[tokio::test(start_paused = true)]
async fn another_message_type_and_back_gives_a_fresh_id() {
    with_clock(async {
        let actors = actors().await;
        let (rule, ready, release) = first_send_held();
        let server = serve(&actors, rule, Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        let first = passing(&model, id, r#"{"n": 1}"#).await;
        model.send_message(id);
        ready.notified().await;
        tokio::time::advance(Limits::DEFAULT.deadline).await;
        sheet_until(&model, |view| !view.sending).await;
        release.notify_one();
        model.choose_send_type(id, DRAFT7);
        let other = model.send_sheet().borrow().clone().unwrap();
        assert_ne!(other.message_id, first.message_id, "a fresh id at the choice");
        let back = passing(&model, id, r#"{"n": 1}"#).await;
        assert_ne!(back.message_id, first.message_id);
        assert_ne!(back.message_id, other.message_id);
    })
    .await;
}

/// Code review 1, finding 5: a choice of message type within the delay of an edit keeps the edit's deadline.
#[tokio::test(start_paused = true)]
async fn a_type_chosen_within_the_delay_of_an_edit_checks_at_the_edits_deadline() {
    with_clock(async {
        let actors = actors().await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let (model, id) = open_form(&server).await;
        sheet_until(&model, |view| view.verdict != Verdict::Checking).await;
        let (held, release) = model.hold_send_check();
        model.edit_send_payload(id, r#"{"n": 1}"#);
        tokio::time::advance(CHECK_DELAY / 2).await;
        model.choose_send_type(id, REQUIRED);
        let early = tokio::task::spawn_blocking(move || (held.recv_timeout(Duration::from_millis(200)).is_ok(), held));
        let (early, held) = early.await.unwrap();
        assert!(!early, "no check before the edit's deadline");
        tokio::time::advance(CHECK_DELAY / 2).await;
        wait_for_hold(held).await;
        release.send(()).unwrap();
        let view = sheet_until(&model, |view| view.verdict != Verdict::Checking).await;
        assert_eq!((view.chosen, view.verdict), (REQUIRED, Verdict::Passes));
        // Without an edit's delay running, a choice checks at once.
        model.choose_send_type(id, ANY);
        assert_eq!(sheet_until(&model, |view| view.verdict != Verdict::Checking).await.verdict, Verdict::Passes);
    })
    .await;
}
