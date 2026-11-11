//! The three plugin hooks: when they fire, what they see, and the order of the plugins of a node.

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use cicada::plugin::{ClaimContext, HandlerErrorContext, HookResult, OutcomeContext, Plugin};
use cicada::prelude::*;
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Hooked {
    id: String,
}

impl ActorRef for Hooked {
    const TYPE: &'static str = "hooked";
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

    async fn on_claim(&self, ctx: &ClaimContext<'_>) -> HookResult {
        let m = ctx.info().message();
        self.0.push(format!("claim:{}", m.message_type));
        ctx.info().attributes().insert(TraceId(format!("t-{}", m.attempt)));
        Ok(())
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
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

/// A node that hosts the hooked type with the tracer installed.
async fn harness(events: &Events) -> TestActors {
    let actor = HookedActor { events: events.clone() };
    let tracer = Tracer(events.clone());
    TestActors::builder().with(move |n| n.host(actor).install(tracer)).build().await.unwrap()
}

#[tokio::test(start_paused = true)]
async fn hooks_fire_in_order_and_on_claim_attributes_reach_the_handler() {
    let events = Events::default();
    let test = harness(&events).await;
    test.create(&h("h"), &HookedState::default()).await;
    test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    assert_eq!(
        events.all(),
        vec!["claim:Work", "handler:t-1", "outcome:Work:Done:1", "claim:Note", "outcome:Note:Done:0"]
    );
}

/// Counts the turns it sees.
#[derive(Default)]
struct Counting(std::sync::atomic::AtomicUsize);

#[async_trait]
impl Plugin for Counting {
    fn name(&self) -> &str {
        "Counting"
    }

    async fn on_outcome(&self, _ctx: &OutcomeContext<'_>) -> HookResult {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn an_arc_of_a_plugin_is_installed_as_a_plugin_and_shares_its_value() {
    let counting = Arc::new(Counting::default());
    let actor = HookedActor { events: Events::default() };
    let installed = counting.clone();
    let test = TestActors::builder().with(move |n| n.host(actor).install(installed)).build().await.unwrap();
    test.create(&h("h"), &HookedState::default()).await;
    test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    assert_eq!(test.run_until_idle().await, 2);
    assert_eq!(counting.0.load(std::sync::atomic::Ordering::SeqCst), 2, "Work and its Note");
}

#[tokio::test(start_paused = true)]
async fn a_handler_failure_stands_and_the_error_and_outcome_hooks_see_it() {
    let events = Events::default();
    let test = harness(&events).await;
    test.create(&h("h"), &HookedState::default()).await;
    let m = test.send(&h("h"), HookedMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().error.as_deref(), Some("work failed"));
    // The handler's state change and send were discarded.
    assert_eq!(test.state::<_, HookedState>(&h("h")).await, HookedState::default());
    assert!(test.pending(&h("h")).await.is_empty());
    assert_eq!(
        events.all(),
        vec!["claim:Work", "handler:t-1", "error:work failed", "outcome:Work:Failed(error=work failed):0"]
    );
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

    async fn on_claim(&self, ctx: &ClaimContext<'_>) -> HookResult {
        self.events.push(self.name.to_lowercase());
        if self.fail_on_note && ctx.info().message().message_type == "Note" {
            anyhow::bail!("claim hook broke");
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn plugins_run_in_installation_order_and_a_failing_on_claim_fails_the_turn() {
    let events = Events::default();
    let actor = HookedActor { events: events.clone() };
    let first = Named { name: "First", events: events.clone(), fail_on_note: false };
    let second = Named { name: "Second", events: events.clone(), fail_on_note: true };
    let third = Named { name: "Third", events: events.clone(), fail_on_note: false };
    let test = TestActors::builder()
        .with(move |n| n.install(first).host(actor).install(second).install(third))
        .build()
        .await
        .unwrap();
    test.create(&h("h"), &HookedState::default()).await;
    test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    // The Note's turn is refused by the second plugin: the third is not reached and no handler runs.
    assert_eq!(events.all(), vec!["first", "second", "third", "handler:null", "first", "second"]);
    let failures = test.failures().await;
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].envelope.message_type, "Note");
    assert_eq!(failures[0].error.as_deref(), Some("on_claim of plugin Second failed: claim hook broke"));
    assert_eq!(test.state::<_, HookedState>(&h("h")).await.notes, vec!["worked"]);
}
