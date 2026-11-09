//! H1: every send of a turn records the turn's message as its cause, and `chain(id)` follows the links both ways.

mod common;

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use cicada::plugin::{BeforeCommitContext, HookResult, HookSend, Plugin};
use cicada::prelude::*;
use cicada::runtime::MessageChain;
use cicada::store::{ChainEntry, MessageId, MessageStatus, OutgoingMessage};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Link {
    id: String,
}

impl ActorRef for Link {
    const TEMPLATE: &'static str = "actor://link/{id}";
    type Message = LinkMsg;
}

fn link(id: &str) -> Link {
    Link { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum LinkMsg {
    /// Creates `width` children and sends each a `Fan` one level less deep, until `depth` is 0.
    Fan { width: u32, depth: u32 },
    /// Sends itself `Hop(remaining - 1)` until `remaining` is 0.
    Hop { remaining: u32 },
    /// Sends nothing; with `later`, sends itself a delayed `Ping` that stays pending.
    Ping {
        #[serde(default)]
        later: bool,
        #[serde(default)]
        fail: bool,
    },
}

/// Sends a `Ping` to self from `before_commit` on every `Hop(0)`, so hook sends are covered.
struct EchoHook;

#[async_trait]
impl Plugin for EchoHook {
    fn name(&self) -> &str {
        "Echo"
    }

    async fn before_commit(&self, ctx: &mut BeforeCommitContext<'_>) -> HookResult {
        let m = ctx.info().message();
        if m.message_type == "Hop" && m.payload == json!({"remaining": 0}) {
            let me = ctx.info().self_address().clone();
            ctx.send(&me, "Ping", json!({}), HookSend::default()).await?;
        }
        Ok(())
    }
}

struct LinkActor;

impl Actor for LinkActor {
    type Ref = Link;
    type State = Stateless;
    type Message = LinkMsg;
    // The custom policy answers an undecodable message with a Ping, so unhandled sends are covered too.
    const UNHANDLED: Unhandled = Unhandled::Custom;

    async fn handle(&self, turn: &mut Turn<Self>, message: LinkMsg) -> HandlerResult {
        match message {
            LinkMsg::Fan { width, depth } => {
                if depth > 0 {
                    for i in 0..width {
                        let child = link(&format!("{}.{i}", turn.self_ref().id));
                        turn.create_stateless(&child)?;
                        turn.send(&child, LinkMsg::Fan { width, depth: depth - 1 })?;
                    }
                }
            }
            LinkMsg::Hop { remaining } => {
                if remaining > 0 {
                    turn.send_self(LinkMsg::Hop { remaining: remaining - 1 })?;
                }
            }
            LinkMsg::Ping { later, fail } => {
                if later {
                    turn.send_self_with(
                        LinkMsg::Ping { later: false, fail: false },
                        SendOptions::new().delay(Duration::from_secs(300)),
                    )?;
                }
                if fail {
                    return Err(HandlerError::msg("ping failed"));
                }
            }
        }
        Ok(())
    }

    async fn on_unhandled(&self, turn: &mut Turn<Self>, _reason: String) -> Result<UnhandledDecision, HandlerError> {
        turn.send_self(LinkMsg::Ping { later: false, fail: false })?;
        Ok(UnhandledDecision::Handled)
    }
}

async fn harness() -> TestActors {
    let test =
        TestActors::builder().with(|n| n.host(ActorType::new(LinkActor).install(EchoHook))).build().await.unwrap();
    test.create_stateless(&link("r")).await;
    test
}

fn status(entry: &ChainEntry) -> &MessageStatus {
    match entry {
        ChainEntry::Live(status) => status,
        other => panic!("not live: {other:?}"),
    }
}

async fn chain(test: &TestActors, id: &MessageId) -> MessageChain {
    test.system().chain(id).await.unwrap().expect("a chain")
}

#[tokio::test(start_paused = true)]
async fn handler_hook_and_unhandled_policy_sends_carry_the_turns_message_as_their_cause() {
    let test = harness().await;
    let hop = test.send(&link("r"), LinkMsg::Hop { remaining: 1 }).await;
    assert!(hop.caused_by.is_none(), "a send from outside an actor has no cause");
    test.run_until_idle().await;
    let whole = chain(&test, &hop.id).await;
    let mut by_type: HashMap<String, Vec<&ChainEntry>> = HashMap::new();
    for e in &whole.entries {
        by_type.entry(status(e).envelope().message_type.clone()).or_default().push(e);
    }
    let hop0: Vec<_> = by_type["Hop"].iter().filter(|e| e.id() != &hop.id).collect();
    assert_eq!(hop0.len(), 1);
    assert_eq!(hop0[0].caused_by(), Some(&hop.id), "handler send");
    assert_eq!(by_type["Ping"].len(), 1);
    assert_eq!(by_type["Ping"][0].caused_by(), Some(hop0[0].id()), "hook send");

    let bad = enqueue(&**test.store(), OutgoingMessage::new(addr(&link("r")), "Hop", json!({"remaining": "x"}))).await;
    test.run_until_idle().await;
    let entries = chain(&test, &bad.id).await.entries;
    let answers: Vec<_> = entries.iter().filter(|e| e.id() != &bad.id).collect();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].caused_by(), Some(&bad.id), "unhandled-policy send");
}

#[tokio::test(start_paused = true)]
async fn a_chain_holds_what_led_to_a_message_and_everything_it_caused() {
    let test = harness().await;
    let root = test.send(&link("r"), LinkMsg::Fan { width: 2, depth: 3 }).await;
    test.run_until_idle().await;

    let whole = chain(&test, &root.id).await;
    assert_eq!(whole.focus, root.id);
    assert_eq!(whole.entries.len(), 1 + 2 + 4 + 8);
    assert!(!whole.truncated);
    assert!(whole.entries.iter().all(|e| matches!(status(e), MessageStatus::Finished(_))));

    // From a leaf: only its ancestors, up to the root; nothing below it, no siblings.
    let entry_of = |chain: &MessageChain, id: &str| {
        chain.entries.iter().find(|e| status(e).envelope().to == addr(&link(id))).unwrap().id().clone()
    };
    let leaf = entry_of(&whole, "r.1.0.1");
    let from_leaf = chain(&test, &leaf).await;
    assert_eq!(from_leaf.focus, leaf);
    let mut targets: Vec<String> = from_leaf.entries.iter().map(|e| status(e).envelope().to.to_string()).collect();
    targets.sort();
    assert_eq!(targets, vec!["actor://link/r", "actor://link/r.1", "actor://link/r.1.0", "actor://link/r.1.0.1"]);
    assert!(!from_leaf.truncated);

    // From the middle: its ancestors and its subtree.
    let middle = entry_of(&whole, "r.0");
    assert_eq!(chain(&test, &middle).await.entries.len(), 1 + 1 + 2 + 4);
}

#[tokio::test(start_paused = true)]
async fn pending_and_failed_messages_are_part_of_a_chain() {
    let test = harness().await;
    let ping = test.send(&link("r"), LinkMsg::Ping { later: true, fail: false }).await;
    test.run_until_idle().await;
    let c = chain(&test, &ping.id).await;
    assert_eq!(c.entries.len(), 2);
    let other: Vec<_> = c.entries.iter().filter(|e| e.id() != &ping.id).collect();
    assert!(matches!(status(other[0]), MessageStatus::Pending(_)));

    let failing = test.send(&link("r"), LinkMsg::Ping { later: true, fail: true }).await;
    test.run_until_idle().await;
    let failed = chain(&test, &failing.id).await;
    assert_eq!(
        failed.entries.iter().map(|e| e.id().clone()).collect::<Vec<_>>(),
        vec![failing.id],
        "a failed turn's sends were discarded"
    );
    assert!(matches!(status(&failed.entries[0]), MessageStatus::Finished(_)));
}

#[tokio::test(start_paused = true)]
async fn a_long_chain_is_cut_off_after_the_maximum_depth_both_ways() {
    let test = harness().await;
    let start = test.send(&link("r"), LinkMsg::Hop { remaining: MessageChain::MAX_DEPTH as u32 + 5 }).await;
    test.run_until_idle().await;
    let down = chain(&test, &start.id).await;
    assert_eq!(down.entries.len(), 1 + MessageChain::MAX_DEPTH);
    assert!(down.truncated);

    let last_hop = |c: &MessageChain| {
        c.entries
            .iter()
            .map(|e| status(e).envelope().clone())
            .filter(|m| m.message_type == "Hop")
            .max_by_key(|m| m.seq)
            .unwrap()
    };
    let last = last_hop(&down);
    let tail = chain(&test, &last.id).await;
    let end = last_hop(&tail);
    let from_end = chain(&test, &end.id).await;
    assert_eq!(from_end.entries.len(), 1 + MessageChain::MAX_DEPTH + 1, "50 causes up and the hook's Ping below");
    assert!(from_end.truncated);
}

#[tokio::test(start_paused = true)]
async fn a_wide_chain_is_cut_off_after_the_maximum_number_of_caused_messages() {
    let test = harness().await;
    let root = test.send(&link("r"), LinkMsg::Fan { width: 25, depth: 2 }).await; // 25 + 625 caused messages
    test.run_until_idle().await;
    let c = chain(&test, &root.id).await;
    assert_eq!(c.entries.len(), 1 + MessageChain::MAX_DESCENDANTS);
    assert!(c.truncated);
    assert_eq!(c.entries.iter().filter(|e| e.caused_by() == Some(&root.id)).count(), 25, "the first level is complete");
}

#[tokio::test(start_paused = true)]
async fn an_unknown_message_has_no_chain() {
    let test = harness().await;
    assert!(test.system().chain(&MessageId::random()).await.unwrap().is_none());
}
