//! The topic (ADR 0028): an actor that keeps subscribers and sends each published message to all of them, each in a
//! turn of its own.

use cicada::message::MessageFamily;
use cicada::prelude::*;
use cicada::turn::TurnError;
use cicada::{Failed, FailureCause, Stopped};
use serde::{Deserialize, Serialize};

use crate::supervisor::retry_policy;
use crate::{EncodeError, StoredMessage};

/// A topic, `cicada.topic/<name>`: the name of the topic.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Topic {
    /// The name of the topic: the key of its address.
    pub name: String,
}

impl Topic {
    /// The topic named `name`.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for Topic {
    const TYPE: &'static str = "cicada.topic";
    type Message = TopicMsg;
}

/// The messages of a [`Topic`]. `Deliver` is its own: it acts on one only when it sent it itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum TopicMsg {
    /// Adds a subscriber, and watches the topic itself.
    Subscribe {
        /// The subscriber.
        subscriber: ActorAddress,
    },
    /// Removes a subscriber.
    Unsubscribe {
        /// The subscriber.
        subscriber: ActorAddress,
    },
    /// Sends the topic one `Deliver` for each subscriber, with the correlation id of the publish.
    Publish {
        /// The message for the subscribers.
        message: StoredMessage,
    },
    /// Sends the message to one subscriber, when it is still a subscriber. Sent by the topic to itself, with a retry
    /// policy.
    Deliver {
        /// The subscriber.
        subscriber: ActorAddress,
        /// The message.
        message: StoredMessage,
    },
    /// A message of the topic itself ended `failed`: a `Deliver` whose commit was rejected drops its subscriber, which
    /// is not there. Only a notification that the store wrote counts: one that a client sent drops nobody.
    Failed(Failed),
    /// Ignored.
    Stopped(Stopped),
}

impl TopicMsg {
    /// A `Publish` of `message`, a message of some family. A value that serde cannot encode, or that has no JSON Schema,
    /// does not compile; a value that is not a message family (an enum with `#[serde(tag = "type")]`) is refused with
    /// [`EncodeError`]. The families of the subscribers cannot be known.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum OrderMsg { Shipped { id: u32 } }
    /// use cicada_actors::TopicMsg;
    ///
    /// let publish = TopicMsg::publish(OrderMsg::Shipped { id: 7 })?;
    /// # Ok::<(), cicada_actors::EncodeError>(())
    /// ```
    ///
    /// A value without serde and a schema does not compile:
    ///
    /// ```compile_fail
    /// use cicada_actors::TopicMsg;
    ///
    /// struct Shipped {
    ///     id: u32,
    /// }
    ///
    /// let publish = TopicMsg::publish(Shipped { id: 7 });
    /// ```
    pub fn publish<M: MessageFamily>(message: M) -> Result<Self, EncodeError> {
        Ok(TopicMsg::Publish { message: StoredMessage::of(&message)? })
    }
}

/// The state of a [`Topic`]: its subscribers, in the order they subscribed, each once.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TopicState {
    /// The subscribers.
    #[serde(default)]
    pub subscribers: Vec<ActorAddress>,
}

/// The topic type. It delivers to each subscriber in a turn of its own, so a subscriber that is gone fails its own
/// delivery and nobody else's; it drops that subscriber when it is told of the failure through the watch on itself. It
/// does not watch its subscribers. A delivery whose send is refused at the call (no registered type, or a type that
/// does not take the message) is refused for good, sends nothing and keeps the subscriber; one whose read of the store
/// fails is tried again. One subscriber gets the messages in the order they were published, but for a delivery that
/// was tried again.
#[derive(Debug, Clone, Copy, Default)]
pub struct TopicActor;

impl Actor for TopicActor {
    type Ref = Topic;
    type State = TopicState;

    async fn handle(&self, turn: &mut Turn<Self>, message: TopicMsg) -> HandlerResult {
        match message {
            TopicMsg::Subscribe { subscriber } => {
                if !turn.state().subscribers.contains(&subscriber) {
                    turn.state_mut().subscribers.push(subscriber);
                }
                let itself = turn.self_address().clone();
                turn.watch(&itself)?;
            }
            TopicMsg::Unsubscribe { subscriber } => {
                if turn.state().subscribers.contains(&subscriber) {
                    turn.state_mut().subscribers.retain(|s| s != &subscriber);
                }
            }
            TopicMsg::Publish { message } => {
                let correlation_id = turn.message().correlation_id.clone();
                for subscriber in turn.state().subscribers.clone() {
                    let mut options = SendOptions::new().retry(retry_policy());
                    options.correlation_id = correlation_id.clone();
                    turn.send_self_with(TopicMsg::Deliver { subscriber, message: message.clone() }, options)?;
                }
            }
            TopicMsg::Deliver { subscriber, message } => {
                let own = turn.message().from.as_ref() == Some(turn.self_address());
                if own && turn.state().subscribers.contains(&subscriber) {
                    deliver(turn, &subscriber, message).await?;
                }
            }
            TopicMsg::Failed(failed) if turn.message().notification => dropped(turn, &failed),
            TopicMsg::Failed(_) => {}
            TopicMsg::Stopped(_) => {}
        }
        Ok(())
    }
}

async fn deliver(turn: &mut Turn<TopicActor>, subscriber: &ActorAddress, message: StoredMessage) -> HandlerResult {
    let mut options = SendOptions::new();
    options.correlation_id = turn.message().correlation_id.clone();
    match turn.send_raw(subscriber, &message.message_type, message.payload, options).await {
        Ok(_) => Ok(()),
        // Tried again by the retry policy of the delivery.
        Err(e @ TurnError::Store(_)) => Err(e.into()),
        // A publisher's mistake must not drop the subscriber: refused for good, and shown.
        Err(e) => Err(HandlerError::permanent_from(e)),
    }
}

/// Drops the subscriber of a `Deliver` that the topic sent itself and whose commit was rejected: it is not there.
fn dropped(turn: &mut Turn<TopicActor>, failed: &Failed) {
    let itself = turn.self_address();
    if &failed.actor != itself || failed.from.as_ref() != Some(itself) || failed.cause != FailureCause::Commit {
        return;
    }
    let Ok(TopicMsg::Deliver { subscriber, .. }) =
        cicada::message::decode::<TopicMsg>(&failed.message_type, &failed.payload)
    else {
        return;
    };
    if turn.state().subscribers.contains(&subscriber) {
        turn.state_mut().subscribers.retain(|s| s != &subscriber);
    }
}
