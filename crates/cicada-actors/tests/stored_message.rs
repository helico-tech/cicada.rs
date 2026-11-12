//! A stored message holds the type name and the payload that the family encodes, and reads back from its wire form.

use cicada::prelude::*;
use cicada_actors::StoredMessage;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Worker {
    id: String,
}

impl ActorRef for Worker {
    const TYPE: &'static str = "worker";
    type Message = WorkerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum WorkerMsg {
    Begin {
        batch: u32,
    },
    #[serde(rename = "halt")]
    Stop,
}

#[test]
fn a_stored_message_holds_the_type_name_and_the_payload_the_family_encodes() {
    let worker = Worker { id: "w1".into() };
    let begin = StoredMessage::for_ref(&worker, WorkerMsg::Begin { batch: 7 }).unwrap();
    let encoded = cicada::message::encode(&WorkerMsg::Begin { batch: 7 }).unwrap();
    assert_eq!((begin.message_type.as_str(), &begin.payload), (encoded.message_type.as_str(), &encoded.payload));
    assert_eq!(serde_json::to_value(&begin).unwrap(), json!({"type": "Begin", "payload": {"batch": 7}}));
    let stop = StoredMessage::for_ref(&worker, WorkerMsg::Stop).unwrap();
    assert_eq!(serde_json::to_value(&stop).unwrap(), json!({"type": "halt", "payload": {}}), "the renamed type name");
    let read: StoredMessage = serde_json::from_value(json!({"type": "Begin", "payload": {"batch": 7}})).unwrap();
    assert_eq!(read, begin);
}

#[test]
fn a_value_that_is_no_message_of_a_family_is_refused() {
    let e = StoredMessage::of(&5_u32).unwrap_err();
    assert!(e.to_string().contains("is not a message family"), "{e}");
}
