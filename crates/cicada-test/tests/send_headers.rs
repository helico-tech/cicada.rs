//! `ClaimContext::send_headers`: headers a plugin adds in `on_claim` to every message the turn sends.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cicada::plugin::{ClaimContext, HookResult, Plugin};
use cicada::prelude::*;
use cicada::turn::TurnError;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Relay {
    id: String,
}

impl ActorRef for Relay {
    const TYPE: &'static str = "relay";
    type Message = RelayMsg;
}

fn relay(id: &str) -> Relay {
    Relay { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum RelayMsg {
    /// Sends `Seen` to `b` with `send`, to itself with `send_self` and to `b` with `send_raw`.
    Fan,
    /// Sends `Seen` to `b` with its own `x-run` header.
    Own,
    /// Sends `Seen` to `b` with `count` headers of its own.
    Many { count: usize },
    /// Records the headers it arrived with.
    Seen { tag: String },
}

struct RelayActor {
    seen: Arc<Mutex<BTreeMap<String, BTreeMap<String, String>>>>,
    errors: Arc<Mutex<Vec<String>>>,
}

fn seen(tag: &str) -> RelayMsg {
    RelayMsg::Seen { tag: tag.into() }
}

impl Actor for RelayActor {
    type Ref = Relay;
    type State = cicada::NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: RelayMsg) -> HandlerResult {
        match message {
            RelayMsg::Fan => {
                turn.send(&relay("b"), seen("send"))?;
                turn.send_self(seen("self"))?;
                let to = relay("b").address().unwrap();
                turn.send_raw(&to, "Seen", serde_json::json!({ "tag": "raw" }), SendOptions::new()).await?;
                turn.send_to(&to, &seen("to"), SendOptions::new()).await?;
            }
            RelayMsg::Own => {
                turn.send_with(&relay("b"), seen("own"), SendOptions::new().header("x-run", "mine"))?;
            }
            RelayMsg::Many { count } => {
                let mut options = SendOptions::new();
                for i in 0..count {
                    options = options.header(format!("h{i}"), "v");
                }
                if let Err(e) = turn.send_with(&relay("b"), seen("many"), options) {
                    assert!(matches!(e, TurnError::Invalid(_)), "{e:?}");
                    self.errors.lock().unwrap().push(e.to_string());
                }
            }
            RelayMsg::Seen { tag } => {
                self.seen.lock().unwrap().insert(tag, turn.message().headers.clone());
            }
        }
        Ok(())
    }
}

/// Adds the given headers to every send of a turn.
struct Adds {
    name: &'static str,
    headers: Vec<(&'static str, &'static str)>,
}

#[async_trait]
impl Plugin for Adds {
    fn name(&self) -> &str {
        self.name
    }

    async fn on_claim(&self, ctx: &ClaimContext<'_>) -> HookResult {
        ctx.send_headers(self.headers.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
        Ok(())
    }
}

type Seen = Arc<Mutex<BTreeMap<String, BTreeMap<String, String>>>>;

async fn harness(plugins: Vec<Adds>) -> (TestActors, Seen, Arc<Mutex<Vec<String>>>) {
    let seen = Seen::default();
    let errors = Arc::new(Mutex::new(Vec::new()));
    let actor = RelayActor { seen: seen.clone(), errors: errors.clone() };
    let test = TestActors::builder()
        .with(move |mut node| {
            node = node.host(actor);
            for plugin in plugins {
                node = node.install(plugin);
            }
            node
        })
        .build()
        .await
        .unwrap();
    test.create(&relay("a"), &cicada::NoState {}).await;
    test.create(&relay("b"), &cicada::NoState {}).await;
    (test, seen, errors)
}

fn run_header() -> Adds {
    Adds { name: "Run", headers: vec![("x-run", "r1")] }
}

#[tokio::test(start_paused = true)]
async fn every_kind_of_send_from_the_handler_carries_the_plugin_header() {
    let (test, seen, _) = harness(vec![run_header()]).await;
    test.send(&relay("a"), RelayMsg::Fan).await;
    test.run_until_idle().await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 4, "{seen:?}");
    for (tag, headers) in seen.iter() {
        assert_eq!(headers.get("x-run").map(String::as_str), Some("r1"), "{tag}: {headers:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_send_keeps_its_own_value_of_a_header_and_still_gets_the_others() {
    let both = Adds { name: "Both", headers: vec![("x-run", "r1"), ("x-other", "o1")] };
    let (test, seen, _) = harness(vec![both]).await;
    test.send(&relay("a"), RelayMsg::Own).await;
    test.run_until_idle().await;
    let headers = seen.lock().unwrap()["own"].clone();
    assert_eq!(headers.get("x-run").map(String::as_str), Some("mine"));
    assert_eq!(headers.get("x-other").map(String::as_str), Some("o1"));
}

#[tokio::test(start_paused = true)]
async fn the_headers_of_two_plugins_both_arrive() {
    let second = Adds { name: "Second", headers: vec![("x-second", "s1")] };
    let (test, seen, _) = harness(vec![run_header(), second]).await;
    test.send(&relay("a"), RelayMsg::Fan).await;
    test.run_until_idle().await;
    let headers = seen.lock().unwrap()["send"].clone();
    assert_eq!(headers.get("x-run").map(String::as_str), Some("r1"));
    assert_eq!(headers.get("x-second").map(String::as_str), Some("s1"));
}

#[tokio::test(start_paused = true)]
async fn a_message_sent_from_outside_does_not_get_the_header() {
    let (test, record, _) = harness(vec![run_header()]).await;
    test.send(&relay("b"), seen("outside")).await;
    test.run_until_idle().await;
    assert!(!record.lock().unwrap()["outside"].contains_key("x-run"));
}

#[tokio::test(start_paused = true)]
async fn plugin_headers_count_toward_the_header_limit() {
    let (test, seen, errors) = harness(vec![run_header()]).await;
    // 63 own headers plus the plugin's one is 64: allowed.
    test.send(&relay("a"), RelayMsg::Many { count: 63 }).await;
    test.run_until_idle().await;
    assert!(errors.lock().unwrap().is_empty());
    assert_eq!(seen.lock().unwrap()["many"].len(), 64);
    // 64 own headers plus the plugin's one is 65: refused.
    test.send(&relay("a"), RelayMsg::Many { count: 64 }).await;
    test.run_until_idle().await;
    let errors = errors.lock().unwrap();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("At most 64 headers"), "{errors:?}");
}
