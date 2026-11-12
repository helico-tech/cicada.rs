use cicada::ActorRef;
use cicada::message::{self, MessageFamily};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::EncodeError;

/// A message an actor keeps to send later (ADR 0028): its type name and its JSON payload, as a store holds a message.
/// The actor that keeps it knows where it goes, so it holds no address. Build one with `StoredMessage::for_ref`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoredMessage {
    /// The type name: the variant of the family.
    #[serde(rename = "type")]
    pub message_type: String,
    /// The payload: the fields of the variant, without the type tag.
    pub payload: Value,
}

impl StoredMessage {
    /// The message `message` for an actor of the ref type of `to`: the compiler checks that `to` takes it, as for a
    /// send. On the wire: `{"type": …, "payload": …}`.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Worker { id: String }
    /// # impl ActorRef for Worker { const TYPE: &'static str = "worker"; type Message = WorkerMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum WorkerMsg { Begin { batch: u32 } }
    /// use cicada_actors::StoredMessage;
    ///
    /// let worker = Worker { id: "w1".into() };
    /// let start = StoredMessage::for_ref(&worker, WorkerMsg::Begin { batch: 7 }).unwrap();
    /// assert_eq!((start.message_type.as_str(), &start.payload), ("Begin", &serde_json::json!({"batch": 7})));
    /// ```
    ///
    /// A message of another family does not compile:
    ///
    /// ```compile_fail
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Worker { id: String }
    /// # impl ActorRef for Worker { const TYPE: &'static str = "worker"; type Message = WorkerMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum WorkerMsg { Begin { batch: u32 } }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum MailerMsg { Send { to: String } }
    /// use cicada_actors::StoredMessage;
    ///
    /// let worker = Worker { id: "w1".into() };
    /// let wrong = StoredMessage::for_ref(&worker, MailerMsg::Send { to: "a@b.c".into() });
    /// ```
    pub fn for_ref<R: ActorRef>(to: &R, message: impl Into<R::Message>) -> Result<Self, EncodeError> {
        let _ = to;
        Self::of(&message.into())
    }

    /// A message of any family, checked only by the actor that gets it: its family must have the type name.
    pub fn of<M: MessageFamily>(message: &M) -> Result<Self, EncodeError> {
        let encoded = message::encode(message)?;
        Ok(Self { message_type: encoded.message_type, payload: encoded.payload })
    }
}
