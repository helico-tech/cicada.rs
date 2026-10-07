//! The four plugin hooks: when they fire, what they can do, node vs type installation (Kotlin `HooksTest`).

mod common;

use std::time::Duration;

use async_trait::async_trait;
use cicada::plugin::{
    BeforeCommitContext, ClaimContext, HandlerErrorContext, HookResult, HookSend, OutcomeContext, Plugin,
};
use cicada::prelude::*;
use cicada::store::{Delivery, OutcomeStatus, TurnOutcome};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Hooked {
    id: String,
}

impl ActorRef for Hooked {
    const TEMPLATE: &'static str = "actor://hooked/{id}";
    type Message = HookedMsg;
}

fn h(id: &str) -> Hooked {
    Hooked { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum HookedMsg {
    Work {
        #[serde(default)]
        fail: bool,
    },
    Note {
        text: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct HookedState {
    #[serde(default)]
    notes: Vec<String>,
}

/// The attribute the tracer sets in `on_claim`.
#[derive(Debug, Clone)]
struct TraceId(String);

struct HookedActor {
    events: Events,
}

impl Actor for HookedActor {
    type Ref = Hooked;
    type State = HookedState;
    type Message = HookedMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: HookedMsg) -> HandlerResult {
        match message {
            HookedMsg::Work { fail } => {
                let trace = turn.attributes().get::<TraceId>().map(|t| t.0);
                self.events.push(format!("handler:{}", opt(&trace)));
                turn.state_mut().notes.push("worked".into());
                turn.send_self(HookedMsg::Note { text: "from handler".into() })?;
                if fail {
                    return Err(HandlerError::msg("work failed"));
                }
            }
            HookedMsg::Note { text } => turn.state_mut().notes.push(text),
        }
        Ok(())
    }
}

struct Tracer(Events);

#[async_trait]
impl Plugin for Tracer {
    fn name(&self) -> &str {
        "Tracer"
    }

    async fn on_claim(&self, ctx: &mut ClaimContext<'_>) -> HookResult {
        let m = ctx.info().message();
        self.0.push(format!("claim:{}", m.message_type));
        ctx.info().attributes().insert(TraceId(format!("t-{}", m.attempt)));
        Ok(())
    }

    async fn before_commit(&self, ctx: &mut BeforeCommitContext<'_>) -> HookResult {
        self.0.push(format!("beforeCommit:{}", ctx.info().message().message_type));
        Ok(())
    }

    async fn on_handler_error(&self, ctx: &mut HandlerErrorContext<'_>) -> HookResult {
        self.0.push(format!("error:{}", ctx.error().reason()));
        Ok(())
    }

    async fn on_outcome(&self, ctx: &OutcomeContext<'_>) -> HookResult {
        self.0.push(format!(
            "outcome:{}:{}:{}",
            ctx.info().message().message_type,
            outcome_text(ctx.outcome()),
            ctx.sent().len()
        ));
        Ok(())
    }
}

async fn harness(events: &Events, plugins: Vec<std::sync::Arc<dyn Plugin>>) -> TestActors {
    let actor = HookedActor { events: events.clone() };
    TestActors::builder()
        .with(move |n| {
            let mut t = ActorType::new(actor);
            for p in plugins {
                t = t.install_shared(p);
            }
            n.host(t)
        })
        .build()
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn hooks_fire_in_order_and_on_claim_attributes_reach_the_handler() {
    let events = Events::default();
    let test = harness(&events, vec![std::sync::Arc::new(Tracer(events.clone()))]).await;
    test.create(&h("h"), &HookedState::default()).await;
    test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    assert_eq!(
        events.all(),
        vec![
            "claim:Work",
            "handler:t-1",
            "beforeCommit:Work",
            "outcome:Work:Done:1",
            "claim:Note",
            "beforeCommit:Note",
            "outcome:Note:Done:0",
        ]
    );
}

struct TakeOver;

#[async_trait]
impl Plugin for TakeOver {
    fn name(&self) -> &str {
        "TakeOver"
    }

    async fn on_handler_error(&self, ctx: &mut HandlerErrorContext<'_>) -> HookResult {
        let me = ctx.info().self_address().clone();
        let options = HookSend { delivery: Delivery::After(Duration::from_secs(5)), ..HookSend::default() };
        ctx.send(&me, "Note", json!({"text": "retry"}), options).await?;
        ctx.set_outcome(TurnOutcome::Done).map_err(anyhow::Error::msg)?;
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn on_handler_error_replaces_the_failure_with_its_own_effects_and_outcome() {
    let events = Events::default();
    let test = harness(&events, vec![std::sync::Arc::new(Tracer(events.clone())), std::sync::Arc::new(TakeOver)]).await;
    test.create(&h("h"), &HookedState::default()).await;
    let m = test.send(&h("h"), HookedMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done);
    // The handler's state change and send were discarded; only the hook's delayed send was committed.
    assert_eq!(test.state::<_, HookedState>(&h("h")).await, HookedState::default());
    assert_eq!(test.pending(&h("h")).await.iter().map(|m| m.message_type.clone()).collect::<Vec<_>>(), vec!["Note"]);
    test.advance_time_by(Duration::from_secs(5)).await;
    assert_eq!(test.state::<_, HookedState>(&h("h")).await.notes, vec!["retry"]);
    assert!(events.all().contains(&"error:work failed".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn without_a_replacing_hook_the_failure_stands() {
    let events = Events::default();
    let test = harness(&events, vec![std::sync::Arc::new(Tracer(events.clone()))]).await;
    test.create(&h("h"), &HookedState::default()).await;
    let m = test.send(&h("h"), HookedMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().error.as_deref(), Some("work failed"));
    assert_eq!(events.last(), "outcome:Work:Failed(error=work failed):0");
}

struct Audit;

#[async_trait]
impl Plugin for Audit {
    fn name(&self) -> &str {
        "Audit"
    }

    async fn before_commit(&self, ctx: &mut BeforeCommitContext<'_>) -> HookResult {
        if ctx.info().message().message_type == "Work" {
            let id = ctx.info().message().id.to_string();
            ctx.send(&addr(&h("audit")), "Note", json!({"text": id}), HookSend::default()).await?;
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn before_commit_effects_join_the_atomic_commit_and_a_bad_one_fails_the_whole_turn() {
    let events = Events::default();
    let test = harness(&events, vec![std::sync::Arc::new(Audit)]).await;
    test.create(&h("h"), &HookedState::default()).await;
    let m = test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    // actor://hooked/audit does not exist: the commit is rejected and nothing of the turn is applied.
    assert!(test.outcome(&m).await.unwrap().error.unwrap().contains("actor://hooked/audit"));
    assert_eq!(test.state::<_, HookedState>(&h("h")).await, HookedState::default());

    test.create(&h("audit"), &HookedState::default()).await;
    let m2 = test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m2).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, HookedState>(&h("audit")).await.notes, vec![m2.id.to_string()]);
}

struct Named {
    name: &'static str,
    events: Events,
    fail_on_note: bool,
}

#[async_trait]
impl Plugin for Named {
    fn name(&self) -> &str {
        self.name
    }

    async fn on_claim(&self, ctx: &mut ClaimContext<'_>) -> HookResult {
        self.events.push(self.name.to_lowercase());
        if self.fail_on_note && ctx.info().message().message_type == "Note" {
            anyhow::bail!("claim hook broke");
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn node_plugins_run_before_type_plugins_and_a_failing_on_claim_fails_the_turn() {
    let events = Events::default();
    let actor = HookedActor { events: events.clone() };
    let type_level = Named { name: "Type", events: events.clone(), fail_on_note: true };
    let node_level = Named { name: "Node", events: events.clone(), fail_on_note: false };
    let test = TestActors::builder()
        .with(move |n| n.host(ActorType::new(actor).install(type_level)).install(node_level))
        .build()
        .await
        .unwrap();
    test.create(&h("h"), &HookedState::default()).await;
    test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    assert_eq!(events.all(), vec!["node", "type", "handler:null", "node", "type"]);
    let failures = test.failures().await;
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].envelope.message_type, "Note");
    assert!(failures[0].error.as_deref().unwrap().starts_with("onClaim of plugin Type failed"));
}
