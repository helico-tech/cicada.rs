//! The actor actions through the in-process HTTP server, on virtual time.

use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada_test::{TestActors, claim_and_abandon};
use serde_json::json;
use tokio::{runtime::Handle, sync::Notify};

use super::soft_refresh::with_clock;
use super::{Counter, Fake, Rule, Server, Work, actors, no_rule, serve};
use crate::actors::actions::{Action, Feedback};
use crate::actors::{ActorPath, Actors, PaneView, Read};
use crate::model::Limits;

fn path(key: &str) -> ActorPath {
    ActorPath::root(ActorAddress::parse(&format!("counter/{key}")).unwrap())
}

async fn create(actors: &TestActors, id: &str) -> Counter {
    let actor = Counter { id: id.into() };
    actors.create(&actor, &json!({})).await;
    actor
}

fn model(server: &Server) -> Actors {
    let model = Actors::new(Handle::current(), Limits::DEFAULT);
    model.use_client(Some(server.client.clone()));
    model
}

async fn select(model: &Actors, key: &str) -> PaneView {
    model.select(Some(path(key)));
    model.pane().wait_for(|pane| !matches!(pane.actor, Read::Loading)).await.unwrap().clone()
}

async fn apply(model: &Actors, pane: &PaneView, action: Action) -> Feedback {
    if let Some(sheet) = model.press_action(pane, action) {
        model.answer_action(sheet.id, true);
    }
    model.actions().wait_for(|v| !v.busy).await.unwrap().feedback.clone().unwrap().1
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
        let pane = select(&model, "a").await;
        let address = path("a");
        for (action, paused) in [(Action::Pause, true), (Action::Resume, false)] {
            assert_eq!(apply(&model, &pane, action).await, Feedback::Succeeded(action, 0));
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
        assert_eq!(apply(&model, &pane, Action::CancelTurn).await, Feedback::Succeeded(Action::CancelTurn, 0));
        assert_eq!(apply(&model, &pane, Action::Purge).await, Feedback::Succeeded(Action::Purge, 2));
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
        assert_eq!(apply(&model, &pane, Action::Delete).await, Feedback::Succeeded(Action::Delete, 0));
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
        assert_eq!(model.actions().borrow().feedback, Some((a_again.generation, Feedback::SelectionChanged)));

        assert!(model.press_action(&a, Action::Pause).is_none());
        assert_eq!(model.actions().borrow().feedback, Some((a_again.generation, Feedback::SelectionChanged)));

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
        let (rule, arrived, release) = held("/actors/counter/a/mailbox/pause");
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
