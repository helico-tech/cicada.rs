//! The event (ADR 0028, ADR 0031): an actor that keeps a list of subscribed flow types and, for each publication,
//! creates and starts one run of each of them in one commit, keyed by the publication's message id.

use std::collections::HashSet;

use cicada::TypeName;
use cicada::prelude::*;
use cicada::turn::TurnError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::supervisor::retry_policy;
use crate::{EncodeError, FlowMsg, FlowState};

/// The most flow types an event may hold.
const MAX_FLOWS: usize = 100;

/// An event, `cicada.event/<name>`: one for each business event whose subscribed flows it starts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Event {
    /// The name of the event: the key of its address.
    pub name: String,
}

impl Event {
    /// The event named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for Event {
    const TYPE: &'static str = "cicada.event";
    type Message = EventMsg;
}

/// The messages of an [`Event`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum EventMsg {
    /// Adds a flow type to the end of the list; a type held already changes nothing, also at the limit of 100. A name
    /// that is no type name, or a 101st type, is refused for good. The registry is not read: the type is checked when
    /// an event is published.
    Subscribe {
        /// The type name of the flow.
        flow: String,
    },
    /// Removes a flow type; one not held changes nothing. A name that is no type name is refused for good. The runs
    /// it started stay.
    Unsubscribe {
        /// The type name of the flow.
        flow: String,
    },
    /// Creates one run of each subscribed flow type with `input`, keyed by this message's id, and sends each a
    /// `Continue`, all in one commit.
    Publish {
        /// The input of each run: any JSON value, `null` included, but not absent.
        input: Value,
    },
    /// A `Publish` whose input is `{"items": [...]}`: one run of each flow type for the whole batch, also an empty
    /// one. A batcher whose message type is `PublishBatch` sends it as it is.
    PublishBatch {
        /// The items of the batch.
        items: Vec<Value>,
    },
}

impl EventMsg {
    /// A `Publish` of `input`. Fails for a value that does not serialize to JSON.
    ///
    /// ```
    /// use cicada_actors::EventMsg;
    ///
    /// let publish = EventMsg::publish(serde_json::json!({"order": 7}))?;
    /// assert_eq!(serde_json::to_value(&publish)?, serde_json::json!({"type": "Publish", "input": {"order": 7}}));
    /// assert_eq!(EventMsg::publish(())?, EventMsg::Publish { input: serde_json::Value::Null }, "null is an input");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn publish(input: impl Serialize) -> Result<Self, EncodeError> {
        let input = serde_json::to_value(input).map_err(|e| EncodeError(e.to_string()))?;
        Ok(EventMsg::Publish { input })
    }

    /// A `PublishBatch` of `items`. Fails for an item that does not serialize to JSON.
    ///
    /// ```
    /// use cicada_actors::EventMsg;
    ///
    /// let batch = EventMsg::publish_batch([1, 2])?;
    /// assert_eq!(serde_json::to_value(&batch)?, serde_json::json!({"type": "PublishBatch", "items": [1, 2]}));
    /// assert_eq!(EventMsg::publish_batch(Vec::<u32>::new())?, EventMsg::PublishBatch { items: vec![] });
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn publish_batch<T: Serialize>(items: impl IntoIterator<Item = T>) -> Result<Self, EncodeError> {
        let items = items
            .into_iter()
            .map(|item| serde_json::to_value(item).map_err(|e| EncodeError(e.to_string())))
            .collect::<Result<_, _>>()?;
        Ok(EventMsg::PublishBatch { items })
    }
}

/// The state of an [`Event`]: the subscribed flow types, in the order they subscribed, each once.
///
/// On the wire: `{"flows":["shop.email","shop.fulfil"]}`; `flows` defaults to empty. A decoded state is checked: at
/// most 100 unique type names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "RawState")]
pub struct EventState {
    /// The type names of the subscribed flows.
    pub flows: Vec<String>,
}

/// The wire form of an [`EventState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
struct RawState {
    /// The type names of the subscribed flows, in the order they subscribed; at most 100, each once; absent: none.
    #[serde(default)]
    #[schemars(
        length(max = 100),
        inner(length(min = 1, max = 100), regex(pattern = r"^[a-z][a-z0-9._-]*$")),
        extend("uniqueItems" = true)
    )]
    flows: Vec<String>,
}

impl TryFrom<RawState> for EventState {
    type Error = String;

    fn try_from(raw: RawState) -> Result<Self, String> {
        if raw.flows.len() > MAX_FLOWS {
            return Err(format!("an event holds at most {MAX_FLOWS} flows, had {}", raw.flows.len()));
        }
        let mut seen = HashSet::new();
        for flow in &raw.flows {
            parse(flow)?;
            if !seen.insert(flow) {
                return Err(format!("the flow `{flow}` is held twice"));
            }
        }
        Ok(Self { flows: raw.flows })
    }
}

fn parse(flow: &str) -> Result<TypeName, String> {
    TypeName::parse(flow).map_err(|e| format!("`{flow}` is no flow type: {e}"))
}

/// The event type. It has no settings and never stops itself; the host creates each event and subscribes its flows,
/// at its start or in the turn that creates the event.
///
/// A publication takes its own message id as the key of every run, so a run of `shop.email` for the publication
/// `event-17` is `shop.email/event-17`; the id is one name of the key, percent-encoded once. For each subscribed type,
/// in order, it creates the run as an independent actor with `{"input": <input>, "journal": [], "status": "running",
/// "armed": null}` and sends it a `Continue` with the crate's shared retry policy, the publication's correlation id and
/// no other header; the run's first turn arms the flow's own policy. The creates and sends commit together, or none of
/// them: an id that is no key (`.` or `..`), a type that is not registered or does not take `Continue`, or a state
/// that a node hosting the type cannot decode, is refused for good (the cause `refused`), and a read of the store that
/// fails is tried by the publication's own retry policy. With no subscriptions it does nothing and needs no key. A run
/// that exists already keeps its state. It installs no watch, and a failure of a run after the commit is the run's.
///
/// Subscribe only flow types: the registry cannot tell a flow from another actor that takes `Continue`. A node that
/// does not host a flow cannot check its input, so a run whose input it cannot decode commits and fails its first
/// message with the cause `state`. The run state holds the input one level down and a batch's items three levels down,
/// so an input may be 63 levels deep and an item 61.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventActor;

impl Actor for EventActor {
    type Ref = Event;
    type State = EventState;

    async fn handle(&self, turn: &mut Turn<Self>, message: EventMsg) -> HandlerResult {
        match message {
            EventMsg::Subscribe { flow } => {
                parse(&flow).map_err(HandlerError::permanent)?;
                let flows = &turn.state().flows;
                if flows.contains(&flow) {
                    return Ok(());
                }
                if flows.len() >= MAX_FLOWS {
                    return Err(HandlerError::permanent(format!(
                        "an event holds at most {MAX_FLOWS} flows; `{flow}` is refused"
                    )));
                }
                turn.state_mut().flows.push(flow);
                Ok(())
            }
            EventMsg::Unsubscribe { flow } => {
                parse(&flow).map_err(HandlerError::permanent)?;
                if turn.state().flows.contains(&flow) {
                    turn.state_mut().flows.retain(|f| f != &flow);
                }
                Ok(())
            }
            EventMsg::Publish { input } => publish(turn, input).await,
            EventMsg::PublishBatch { items } => publish(turn, serde_json::json!({ "items": items })).await,
        }
    }
}

/// Creates and starts one run of each subscribed flow with `input`, keyed by the id of this turn's message. On an
/// error the turn rolls back, so nothing of it commits.
async fn publish(turn: &mut Turn<EventActor>, input: Value) -> HandlerResult {
    let flows = turn.state().flows.clone();
    if flows.is_empty() {
        return Ok(());
    }
    let key = turn.message().id.as_str().to_owned();
    let state = serde_json::to_value(FlowState::new(input)).map_err(HandlerError::permanent_from)?;
    let correlation_id = turn.message().correlation_id.clone();
    for flow in flows {
        let type_name = parse(&flow).map_err(HandlerError::permanent)?;
        let address = ActorAddress::of(&type_name, &key)
            .map_err(|e| HandlerError::permanent(format!("the publication id `{key}` cannot key a run: {e}")))?;
        refused_for_good(turn.create_raw(&address, state.clone()).await)?;
        let mut options = SendOptions::new().retry(retry_policy());
        options.correlation_id = correlation_id.clone();
        refused_for_good(turn.send_to(&address, &FlowMsg::Continue, options).await.map(drop))?;
    }
    Ok(())
}

/// A failed read of the store is tried again by the publication's retry policy; anything else refused at the call is
/// refused for good.
fn refused_for_good(result: Result<(), TurnError>) -> HandlerResult {
    match result {
        Ok(()) => Ok(()),
        Err(e @ TurnError::Store(_)) => Err(e.into()),
        Err(e) => Err(HandlerError::permanent_from(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(json: Value) -> Result<EventState, serde_json::Error> {
        serde_json::from_value(json)
    }

    fn names(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("shop.flow{i}")).collect()
    }

    #[test]
    fn the_minimal_state_is_empty_and_ignores_unknown_fields() {
        assert_eq!(decode(serde_json::json!({})).unwrap(), EventState::default());
        let state = decode(serde_json::json!({"flows": ["shop.email", "shop.fulfil"], "other": 1})).unwrap();
        assert_eq!(state.flows, ["shop.email", "shop.fulfil"], "in subscription order");
        assert_eq!(serde_json::to_value(&state).unwrap(), serde_json::json!({"flows": ["shop.email", "shop.fulfil"]}));
    }

    #[test]
    fn a_state_is_checked_for_its_names_and_their_count() {
        assert_eq!(decode(serde_json::json!({ "flows": names(MAX_FLOWS) })).unwrap().flows.len(), MAX_FLOWS);
        for (flows, why) in [
            (serde_json::json!(names(MAX_FLOWS + 1)), "at most 100 flows, had 101"),
            (serde_json::json!(["shop.email", "shop.email"]), "held twice"),
            (serde_json::json!(["Shop"]), "is no flow type"),
            (serde_json::json!([""]), "is no flow type"),
            (serde_json::json!(["shop/email"]), "is no flow type"),
            (serde_json::json!(["a".repeat(TypeName::MAX_LENGTH + 1)]), "is no flow type"),
        ] {
            let decoded = decode(serde_json::json!({ "flows": flows }));
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {why}");
        }
        assert!(decode(serde_json::json!({"flows": "shop.email"})).is_err(), "a list");
        assert!(decode(serde_json::json!({"flows": null})).is_err(), "not null");
    }

    #[test]
    fn publications_need_their_fields() {
        let decoded = |json: Value| cicada::message::decode::<EventMsg>(json["type"].as_str().unwrap(), &json);
        for input in [serde_json::json!({"a": 1}), serde_json::json!([1]), serde_json::json!(7), Value::Null] {
            let json = serde_json::json!({"type": "Publish", "input": input});
            assert_eq!(decoded(json).unwrap(), EventMsg::Publish { input: input.clone() }, "{input}");
        }
        assert!(decoded(serde_json::json!({"type": "Publish"})).is_err(), "a missing input is no null");
        assert_eq!(
            decoded(serde_json::json!({"type": "PublishBatch", "items": []})).unwrap(),
            EventMsg::PublishBatch { items: vec![] }
        );
        for bad in
            [serde_json::json!({"type": "PublishBatch"}), serde_json::json!({"type": "PublishBatch", "items": 1})]
        {
            assert!(decoded(bad.clone()).is_err(), "{bad}");
        }
        assert!(decoded(serde_json::json!({"type": "Subscribe"})).is_err());
    }

    #[test]
    fn helpers_fail_for_values_that_do_not_serialize() {
        let unencodable = std::collections::HashMap::from([((1u8, 2u8), 3u8)]);
        assert!(EventMsg::publish(&unencodable).is_err());
        assert!(EventMsg::publish_batch([&unencodable]).is_err());
        let deep = (0..70).fold(Value::Null, |inner, _| Value::Array(vec![inner]));
        assert_eq!(EventMsg::publish(&deep).unwrap(), EventMsg::Publish { input: deep.clone() }, "no depth check");
    }

    #[test]
    fn publication_ids_are_one_name_of_the_key() {
        let flow = TypeName::parse("shop.email").unwrap();
        for (id, key) in [("event-17", "event-17"), ("a/b", "a%2Fb"), ("a%2Fb", "a%252Fb"), ("a:b", "a:b")] {
            assert_eq!(ActorAddress::of(&flow, id).unwrap().key(), key, "{id}");
        }
        for id in [".", ".."] {
            assert!(ActorAddress::of(&flow, id).is_err(), "{id} is no key");
        }
    }
}
