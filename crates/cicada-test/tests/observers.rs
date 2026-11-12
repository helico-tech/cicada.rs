//! Turn observers through the public API: when they are told, what they see, the order of the observers of a node,
//! and that one that panics has no say.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
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

struct HookedActor {
    events: Events,
}

impl Actor for HookedActor {
    type Ref = Hooked;
    type State = HookedState;

    async fn handle(&self, turn: &mut Turn<Self>, message: HookedMsg) -> HandlerResult {
        match message {
            HookedMsg::Work { fail } => {
                self.events.push(format!("handler:{}", opt(&turn.message().headers.get("trace"))));
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

/// Writes the start and the end of every turn, and adds the header `trace` with the attempt.
struct Tracer(Events);

struct TracedTurn {
    events: Events,
    message_type: String,
}

impl TurnObserver for Tracer {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        let (message_type, attempt) = (turn.message().message_type.clone(), turn.message().attempt);
        self.0.push(format!("start:{message_type}"));
        turn.add_send_header("trace", format!("t-{attempt}"));
        Box::new(TracedTurn { events: self.0.clone(), message_type })
    }
}

impl ObservedTurn for TracedTurn {
    fn turn_ended(self: Box<Self>, end: &TurnEnd<'_>) {
        let sent = end.sent().iter().map(|m| opt(&m.headers.get("trace"))).collect::<Vec<_>>().join(",");
        self.events.push(format!("end:{}:{}:[{sent}]", self.message_type, outcome_text(end.outcome())));
    }
}

/// A node that hosts the hooked type with the tracer.
async fn harness(events: &Events) -> TestActors {
    let actor = HookedActor { events: events.clone() };
    let tracer = Tracer(events.clone());
    TestActors::builder().with(move |n| n.host(actor).observe(tracer)).build().await.unwrap()
}

#[tokio::test(start_paused = true)]
async fn an_observer_is_told_each_start_and_end_and_its_header_reaches_the_sends() {
    let events = Events::default();
    let test = harness(&events).await;
    test.create(&h("h"), &HookedState::default()).await;
    test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    assert_eq!(
        events.all(),
        vec!["start:Work", "handler:null", "end:Work:Done:[t-1]", "start:Note", "end:Note:Done:[]"],
        "the handler reads nothing an observer made; the message it sent carries the header"
    );
}

/// Counts the turns it is told the end of.
#[derive(Default)]
struct Counting(Arc<AtomicUsize>);

impl TurnObserver for Counting {
    fn turn_started(&self, _turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        Box::new(Counted(self.0.clone()))
    }
}

struct Counted(Arc<AtomicUsize>);

impl ObservedTurn for Counted {
    fn turn_ended(self: Box<Self>, _end: &TurnEnd<'_>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn an_arc_of_an_observer_is_an_observer_and_shares_its_value() {
    let counting = Arc::new(Counting::default());
    let actor = HookedActor { events: Events::default() };
    let added = counting.clone();
    let test = TestActors::builder().with(move |n| n.host(actor).observe(added)).build().await.unwrap();
    test.create(&h("h"), &HookedState::default()).await;
    test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    assert_eq!(test.run_until_idle().await, 2);
    assert_eq!(counting.0.load(Ordering::SeqCst), 2, "Work and its Note");
}

#[tokio::test(start_paused = true)]
async fn a_handler_failure_stands_and_the_observer_is_told_it() {
    let events = Events::default();
    let test = harness(&events).await;
    test.create(&h("h"), &HookedState::default()).await;
    let m = test.send(&h("h"), HookedMsg::Work { fail: true }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().error.as_deref(), Some("work failed"));
    // The handler's state change and send were discarded.
    assert_eq!(test.state::<_, HookedState>(&h("h")).await, HookedState::default());
    assert!(test.pending(&h("h")).await.is_empty());
    assert_eq!(events.all(), vec!["start:Work", "handler:null", "end:Work:Failed(error=work failed):[]"]);
}

/// Writes its name when a turn starts, and panics there for a `Note` when `panics_on_note` is set.
struct Named {
    name: &'static str,
    events: Events,
    panics_on_note: bool,
}

impl TurnObserver for Named {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        self.events.push(self.name);
        if self.panics_on_note && turn.message().message_type == "Note" {
            panic!("the observer broke");
        }
        Box::new(())
    }
}

#[tokio::test(start_paused = true)]
async fn observers_are_called_in_the_order_they_were_added_and_one_that_panics_has_no_say() {
    let events = Events::default();
    let actor = HookedActor { events: events.clone() };
    let first = Named { name: "first", events: events.clone(), panics_on_note: false };
    let second = Named { name: "second", events: events.clone(), panics_on_note: true };
    let third = Named { name: "third", events: events.clone(), panics_on_note: false };
    let test = TestActors::builder()
        .with(move |n| n.observe(first).host(actor).observe(second).observe(third))
        .build()
        .await
        .unwrap();
    test.create(&h("h"), &HookedState::default()).await;
    test.send(&h("h"), HookedMsg::Work { fail: false }).await;
    test.run_until_idle().await;
    // The second panics at the start of the Note's turn: the third is still called and the turn runs.
    assert_eq!(events.all(), vec!["first", "second", "third", "handler:null", "first", "second", "third"]);
    assert!(test.failures().await.is_empty());
    assert_eq!(test.state::<_, HookedState>(&h("h")).await.notes, vec!["worked", "from handler"]);
}
