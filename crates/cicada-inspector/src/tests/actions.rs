//! The actor actions through the in-process HTTP server, on virtual time.

mod reconcile;

use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada_test::{TestActors, claim_and_abandon};
use serde_json::json;
use tokio::{runtime::Handle, sync::Notify};

use super::soft_refresh::with_clock;
use super::{Counter, Fake, Rule, Server, Work, actors, config, no_rule, serve};
use crate::actors::actions::{Action, Feedback};
use crate::actors::{ActorPath, Actors, PaneView, Read};
use crate::model::{Limits, Session};
use axum::http::StatusCode;

fn path(key: &str) -> ActorPath {
    ActorPath::root(ActorAddress::parse(&format!("counter/{key}")).unwrap())
}

pub(super) async fn create(actors: &TestActors, id: &str) -> Counter {
    let actor = Counter { id: id.into() };
    actors.create(&actor, &json!({})).await;
    actor
}

pub(super) fn model(server: &Server) -> Actors {
    let model = Actors::new(Handle::current(), Limits::DEFAULT);
    model.use_client(Some(server.client.clone()));
    model
}

pub(super) async fn select(model: &Actors, key: &str) -> PaneView {
    model.select(Some(path(key)));
    model.pane().wait_for(|pane| !matches!(pane.actor, Read::Loading)).await.unwrap().clone()
}

async fn apply(model: &Actors, pane: &PaneView, action: Action) -> Feedback {
    if let Some(sheet) = model.press_action(pane, action) {
        model.answer_action(sheet.id, true);
    }
    model.actions().wait_for(|v| !v.busy).await.unwrap().feedback.clone().unwrap()
}

async fn assert_notice(model: &Actors, pane: &PaneView, action: Action, expected: &str) {
    let feedback = apply(model, pane, action).await;
    assert_eq!(feedback, notice(model.pane().borrow().generation, expected));
}

fn notice(generation: u64, text: &str) -> Feedback {
    Feedback::Notice { generation, text: text.into() }
}

fn held(route: &'static str) -> (Rule, Arc<Notify>, Arc<Notify>) {
    let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let wait = Fake::Wait { arrived: arrived.clone(), release: release.clone() };
    (Arc::new(move |target| (target == route).then(|| wait.clone())), arrived, release)
}

#[tokio::test(start_paused = true)]
async fn pause_and_resume_write_the_mailbox_flag() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let model = model(&server);
        let address = path("a");
        for (action, paused) in [(Action::Pause, true), (Action::Resume, false)] {
            let pane = select(&model, "a").await;
            let text = if paused { "paused" } else { "resumed" };
            assert_notice(&model, &pane, action, text).await;
            assert_eq!(server.client.get_actor(address.address()).await.unwrap().unwrap().paused, paused);
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn cancel_records_the_request_and_purge_spares_the_running_message() {
    with_clock(async {
        let actors = actors().await;
        let actor = create(&actors, "a").await;
        let running = actors.send(&actor, Work::Ok).await;
        for _ in 0..2 {
            actors.send(&actor, Work::Ok).await;
        }
        let hosted = actors.system().hosted_types();
        claim_and_abandon(actors.store(), &hosted, Duration::from_secs(60), 1).await.unwrap();
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let model = model(&server);
        select(&model, "a").await;
        let pane = model.pane().wait_for(|pane| matches!(pane.turn, Read::Ready(Some(_)))).await.unwrap().clone();
        assert_notice(&model, &pane, Action::CancelTurn, "cancel requested").await;
        assert_notice(&model, &pane, Action::Purge, "purged 2 messages").await;
        let address = path("a");
        let turn = server.client.turn(address.address()).await.unwrap().unwrap();
        assert!(turn.cancel_requested);
        assert_eq!(turn.message_id, running.id);
        let mailbox = server.client.mailbox(address.address(), None, Some(50)).await.unwrap().unwrap();
        assert_eq!(mailbox.items.len(), 1);
        assert_eq!(mailbox.items[0].message.id, running.id);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn delete_removes_the_actor() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        let model = model(&server);
        let pane = select(&model, "a").await;
        assert_notice(&model, &pane, Action::Delete, "deleted").await;
        assert!(server.client.get_actor(path("a").address()).await.unwrap().is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_stale_button_or_sheet_sends_nothing_even_after_reselecting_the_address() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        create(&actors, "b").await;
        let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
        let model = model(&server);
        let a = select(&model, "a").await;
        let sheet = model.press_action(&a, Action::Purge).unwrap();
        let b = select(&model, "b").await;
        assert!(model.actions().borrow().busy);
        assert!(model.press_action(&b, Action::Delete).is_none());
        let a_again = select(&model, "a").await;
        model.answer_action(sheet.id, true);
        assert_eq!(
            model.actions().borrow().feedback,
            Some(notice(a_again.generation, "selection changed; nothing done"))
        );

        assert!(model.press_action(&a, Action::Pause).is_none());
        assert_eq!(
            model.actions().borrow().feedback,
            Some(notice(a_again.generation, "selection changed; nothing done"))
        );

        model.press_action(&a_again, Action::Pause);
        assert!(model.actions().borrow().busy);
        model.select(Some(path("b")));
        let new_generation = model.pane().borrow().generation;
        let before_send = model.actions().wait_for(|view| !view.busy).await.unwrap().feedback.clone();
        assert_eq!(before_send, Some(notice(new_generation, "selection changed; nothing done")));

        while let Ok(event) = server.events.try_recv() {
            assert!(!event.contains("/mailbox/purge") && !event.contains("/mailbox/pause"), "{event}");
        }
        assert!(!server.client.get_actor(path("a").address()).await.unwrap().unwrap().paused);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_slot_extends_through_the_answer_across_a_reselection() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        create(&actors, "b").await;
        let (rule, arrived, release) = held("/actors/counter/a/mailbox/purge");
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        let a = select(&model, "a").await;
        let sheet = model.press_action(&a, Action::Purge).unwrap();
        model.answer_action(sheet.id, true);
        arrived.notified().await;
        let b = select(&model, "b").await;
        assert!(model.press_action(&b, Action::Delete).is_none());
        assert!(model.actions().borrow().busy);
        release.notify_one();
        assert_eq!(model.actions().wait_for(|v| !v.busy).await.unwrap().feedback, None);
        let sheet = model.press_action(&b, Action::Delete).unwrap();
        model.answer_action(sheet.id, false);
        assert!(!model.actions().borrow().busy);
        assert!(server.client.get_actor(path("b").address()).await.unwrap().is_some());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn switching_context_or_closing_drops_the_pending_result() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let late = Fake::Later {
            arrived: arrived.clone(),
            release: release.clone(),
            status: StatusCode::FORBIDDEN,
            body: json!({"type": "urn:example:refusal", "status": 403, "title": "Denied"}),
        };
        let rule: Rule = Arc::new(move |target| (target == "/actors/counter/a/mailbox/pause").then(|| late.clone()));
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        for close in [false, true] {
            let pane = select(&model, "a").await;
            model.press_action(&pane, Action::Pause);
            arrived.notified().await;
            if close {
                model.stop();
            } else {
                model.use_client(Some(server.client.clone()));
            }
            release.notify_one();
            for task in model.take_aborted() {
                assert!(task.await.is_err_and(|e| e.is_cancelled()));
            }
            assert_eq!(*model.actions().borrow(), Default::default());
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn actor_and_turn_absences_leave_the_pane_as_it_was() {
    with_clock(async {
        let actors = actors().await;
        let actor = create(&actors, "a").await;
        let other = create(&actors, "b").await;
        actors.send(&actor, Work::Ok).await;
        let hosted = actors.system().hosted_types();
        claim_and_abandon(actors.store(), &hosted, Duration::from_secs(60), 1).await.unwrap();
        let server = serve(&actors, no_rule(), Duration::ZERO).await;
        server.client.pause_mailbox(path("b").address()).await.unwrap();
        let model = model(&server);
        select(&model, "a").await;
        let a = model.pane().wait_for(pane_read).await.unwrap().clone();
        let text = sheet_text(&model, &a, Action::CancelTurn);
        let Read::Ready(Some(turn)) = &a.turn else { panic!("the turn was read") };
        assert!(text.contains(&format!("counter/a (message {} as last read)", turn.message_id)));
        assert!(text.contains("whichever turn is running when it receives this"));

        server.client.delete(&actor).await.unwrap();
        for action in [Action::Pause, Action::Purge, Action::Delete, Action::CancelTurn] {
            let expected = if action == Action::CancelTurn { "no turn was running" } else { "actor not found" };
            assert_notice(&model, &a, action, expected).await;
            assert_eq!(*model.pane().borrow(), a);
        }

        select(&model, "b").await;
        let b = model.pane().wait_for(pane_read).await.unwrap().clone();
        server.client.delete(&other).await.unwrap();
        assert_notice(&model, &b, Action::Resume, "actor not found").await;
        assert_eq!(*model.pane().borrow(), b);
    })
    .await;
}

fn pane_read(pane: &PaneView) -> bool {
    !matches!(pane.actor, Read::Loading)
        && !matches!(pane.turn, Read::Loading)
        && !matches!(pane.mailbox, Read::Loading)
        && !matches!(pane.watching, Read::Loading)
        && !matches!(pane.watched_by, Read::Loading)
}

fn sheet_text(model: &Actors, pane: &PaneView, action: Action) -> String {
    let sheet = model.press_action(pane, action).unwrap();
    assert_eq!(sheet.action, action);
    model.answer_action(sheet.id, false);
    sheet.text
}

#[tokio::test(start_paused = true)]
async fn confirmations_use_the_loaded_counts_and_distinguish_unread_or_partial_pages() {
    with_clock(async {
        let actors = actors().await;
        let actor = create(&actors, "a").await;
        for _ in 0..51 {
            actors.send(&actor, Work::Ok).await;
        }
        claim_and_abandon(actors.store(), &actors.system().hosted_types(), Duration::from_secs(60), 1).await.unwrap();
        let rule: Rule = Arc::new(|target| {
            (target == "/actors/counter/a/children?limit=100")
                .then(|| Fake::Answer(StatusCode::OK, json!({"items": ["counter/b", "counter/c"], "next": "more"})))
        });
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        select(&model, "a").await;
        let mut pane = model.pane().wait_for(pane_read).await.unwrap().clone();
        assert_eq!(pane.pending().count(), 49);
        let purge = |pane: &PaneView| sheet_text(&model, pane, Action::Purge);
        assert_eq!(
            purge(&pane),
            concat!(
                "Purge the mailbox of counter/a (at least 49 pending, as last read)? ",
                "The running message, if any, is not purged.",
            )
        );
        let Read::Ready(mailbox) = &mut pane.mailbox else { panic!("the mailbox was read") };
        mailbox.more = false;
        assert!(purge(&pane).contains("(49 pending, as last read)"));
        pane.errors.mailbox = Some("503".into());
        assert!(purge(&pane).contains("(not read)"));
        pane.mailbox = Read::Loading;
        assert!(purge(&pane).contains("(not read)"));
        pane.mailbox = Read::Failed("503".into());
        assert!(purge(&pane).contains("(not read)"));

        let delete = |pane: &PaneView| sheet_text(&model, pane, Action::Delete);
        assert!(delete(&pane).contains("(children not read)"));
        model.show();
        let mut list = model.list();
        let generation = list.wait_for(|list| !list.rows.is_empty()).await.unwrap().generation;
        model.open(generation, &path("a"));
        list.wait_for(|list| list.children.get(&path("a")).is_some_and(|page| page.more)).await.unwrap();
        assert_eq!(
            delete(&pane),
            concat!(
                "Delete counter/a (at least 2 children)? ",
                "This removes this actor and every descendant, with all their mailboxes.",
            )
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn refusals_use_sanitized_problem_text_or_only_the_status_and_keep_the_pane() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        let title = format!("Refused\u{0000} secret-token {} suffix", "λ".repeat(520));
        let detail = "Bad\u{001b} request: http://someone:password@node, Bearer token-value, header-secret. \
                      admin may retry 1 time. Password p#; encoded p%23; key xy.";
        let rule: Rule = Arc::new(move |target| match target {
            "/actors/counter/a/mailbox/pause" => Some(Fake::Answer(
                StatusCode::FORBIDDEN,
                json!({"type": "urn:example:refusal", "status": 403, "title": title, "detail": detail}),
            )),
            "/actors/counter/a/mailbox/purge" => {
                Some(Fake::Answer(StatusCode::UNAUTHORIZED, json!({"raw": "an arbitrary error body with a secret"})))
            }
            _ => None,
        });
        let server = serve(&actors, rule, Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let basic = server.url.replacen("://", "://admin:p%23@", 1);
        let mut env = config(&dir, &[("a", &basic)]);
        env.insert("CICADA_TOKEN".into(), "secret-token".into());
        cicada_app::ContextFile::update(&env, |file| {
            let mut entry = file.context("a").unwrap();
            entry.headers.insert("X-Custom".into(), "header-secret".into());
            entry.headers.insert("X-Version".into(), "1".into());
            entry.headers.insert("X-API-Key".into(), "xy".into());
            file.set_context("a", entry);
            Ok(())
        })
        .unwrap();
        let session = Session::new(env, Handle::current(), Limits::DEFAULT);
        session.use_context(Some("a".into()));
        let model = session.actors();
        select(model, "a").await;
        let pane = model.pane().wait_for(pane_read).await.unwrap().clone();

        let Feedback::Alert(alert) = apply(model, &pane, Action::Pause).await else { panic!("expected an alert") };
        assert_eq!(alert.title.chars().count(), 500);
        assert!(alert.title.starts_with("Refused  λ"));
        assert_eq!(
            alert.detail.as_deref(),
            Some("Bad request: http://node, , . admin may retry 1 time. Password ; encoded ; key .")
        );
        assert_eq!(*model.pane().borrow(), pane);
        let Feedback::Alert(alert) = apply(model, &pane, Action::Purge).await else { panic!("expected an alert") };
        assert_eq!((alert.title.as_str(), alert.detail), ("HTTP 401", None));
        assert_eq!(*model.pane().borrow(), pane);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_late_refusal_survives_reselection_and_manual_refresh() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        create(&actors, "b").await;
        let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let late = Fake::Later {
            arrived: arrived.clone(),
            release: release.clone(),
            status: StatusCode::FORBIDDEN,
            body: json!({
                "type": "urn:example:refusal", "status": 403, "title": "Purge refused", "detail": "Missing permission"
            }),
        };
        let rule: Rule = Arc::new(move |target| (target == "/actors/counter/a/mailbox/purge").then(|| late.clone()));
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        for refresh in [false, true] {
            let a = select(&model, "a").await;
            let sheet = model.press_action(&a, Action::Purge).unwrap();
            model.answer_action(sheet.id, true);
            arrived.notified().await;
            if refresh {
                model.refresh();
                model.reloaded().await;
            } else {
                select(&model, "b").await;
            }
            let pane = model.pane().wait_for(pane_read).await.unwrap().clone();
            assert_ne!(pane.generation, a.generation);
            release.notify_one();

            let view = model.actions().wait_for(|view| !view.busy).await.unwrap().clone();
            let Some(Feedback::Alert(alert)) = view.feedback else { panic!("expected a refusal, got {view:?}") };
            assert_eq!(alert.title, "Purge refused");
            assert_eq!(alert.detail.as_deref(), Some("Missing permission"));
            assert_eq!(*model.pane().borrow(), pane);
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_late_absence_after_reselection_names_its_address() {
    with_clock(async {
        let actors = actors().await;
        create(&actors, "a").await;
        create(&actors, "b").await;
        let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let late = Fake::Later {
            arrived: arrived.clone(),
            release: release.clone(),
            status: StatusCode::NOT_FOUND,
            body: json!({"type": "urn:cicada:problem:no-such-instance", "title": "Not Found", "status": 404}),
        };
        let rule: Rule = Arc::new(move |target| (target == "/actors/counter/a/mailbox/purge").then(|| late.clone()));
        let server = serve(&actors, rule, Duration::ZERO).await;
        let model = model(&server);
        let a = select(&model, "a").await;
        let sheet = model.press_action(&a, Action::Purge).unwrap();
        model.answer_action(sheet.id, true);
        arrived.notified().await;
        let b = select(&model, "b").await;
        release.notify_one();

        let view = model.actions().wait_for(|view| !view.busy).await.unwrap().clone();
        let Some(Feedback::Alert(alert)) = view.feedback else { panic!("expected an alert, got {view:?}") };
        assert_eq!((alert.title.as_str(), alert.detail.as_deref()), ("actor not found", Some("counter/a")));
        assert_eq!(model.pane().borrow().generation, b.generation);
    })
    .await;
}
