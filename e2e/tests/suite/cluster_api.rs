//! The HTTP contract end to end on the PostgreSQL cluster (three hosting nodes, one
//! send-only node), driven over HTTP only. Status codes are asserted on raw responses; the typed client is used for
//! everything else.

use std::collections::BTreeMap;

use chrono::{TimeDelta, Utc};
use cicada::api::{MessageState, problem_types};
use cicada::store::MessageId;
use cicada::{ActorAddress, SendOptions};
use cicada_client::CancelOutcome;
use cicada_e2e::cluster::{address, cluster, eventually, path, run_id, uri};
use cicada_e2e::{Class, step};
use cicada_sample_service::e2e::{
    Counter, CounterMsg, CounterState, Recorder, RecorderMsg, RecorderState, Slow, SlowState,
};
use serde_json::json;

use crate::{millis, secs};

pub fn class() -> Class {
    Class::new(
        "ClusterApiTest",
        vec![
            step!(health_on_every_node),
            step!(types_with_json_schemas_on_every_node_and_the_type_of_one_uri),
            step!(create_is_idempotent_get_returns_state_and_version),
            step!(list_by_prefix_with_paging),
            step!(send_through_a_hosting_node_and_through_the_send_only_node_pending_then_done),
            step!(a_delayed_send_is_never_delivered_early),
            step!(an_idempotent_resend_with_the_same_id_has_one_outcome),
            step!(unknown_type_and_bad_payloads),
            step!(a_send_to_an_absent_instance_is_404),
            step!(cancel_a_pending_delayed_message_409_once_finished),
            step!(delete_purges_the_mailbox),
        ],
    )
}

fn step_msg(seq: i64) -> CounterMsg {
    CounterMsg::Step { seq, hold_millis: 0 }
}

fn record(label: &str) -> RecorderMsg {
    RecorderMsg::Record { label: label.to_owned(), sleep_millis: 0 }
}

async fn health_on_every_node() {
    for node in cluster().nodes() {
        let response = node.get("/health").await;
        assert_eq!(response.status, 200, "{node}: {response}");
        assert_eq!(response.text("/node").as_deref(), Some(node.id.as_str()));
        assert_eq!(response.text("/status").as_deref(), Some("up"));
        assert!(node.client.health().await.unwrap());
    }
}

async fn types_with_json_schemas_on_every_node_and_the_type_of_one_uri() {
    let c = cluster();
    let expected = [
        "actor://e2e/slow/{id}",
        "actor://e2e/counter/{id}",
        "actor://e2e/recorder/{id}",
        "actor://e2e/flaky/{id}",
        "actor://orders/{id}",
        "actor://inventory/{sku}",
        "actor://payments",
        "actor://carts/{id}",
        "actor://pricing",
    ];
    let mut hashes = Vec::new();
    for node in c.nodes() {
        let types: BTreeMap<String, _> =
            node.client.types().await.unwrap().into_iter().map(|t| (t.template.clone(), t)).collect();
        for template in expected {
            assert!(types.contains_key(template), "{node} serves {:?}", types.keys());
        }
        let slow = &types["actor://e2e/slow/{id}"];
        assert!(slow.hash.starts_with("sha256:"));
        let sleep = &slow.messages["Sleep"];
        assert!(sleep.pointer("/properties/millis").is_some(), "Sleep schema: {sleep}");
        assert!(slow.state.as_ref().and_then(|s| s.get("properties")).is_some(), "state schema: {:?}", slow.state);
        assert!(types["actor://payments"].state.is_none(), "a stateless type has no state schema");
        hashes.push(types.into_iter().map(|(template, t)| (template, t.hash)).collect::<BTreeMap<_, _>>());
    }
    // Every node answers from the shared registry, also the send-only one that hosts nothing.
    assert!(hashes.windows(2).all(|w| w[0] == w[1]), "nodes disagree on the registry: {hashes:?}");

    let matched = c.send_only.get("/types?uri=actor://e2e/slow/x-1").await;
    assert_eq!(matched.status, 200, "{matched}");
    assert_eq!(matched.text("/type/template").as_deref(), Some("actor://e2e/slow/{id}"));
    assert_eq!(matched.text("/params/id").as_deref(), Some("x-1"));
    let order = c.node2.client.type_of(&ActorAddress::parse("actor://orders/o-9").unwrap()).await.unwrap().unwrap();
    assert_eq!(order.params.get("id"), Some("o-9"));
    assert_eq!(c.node1.get("/types?uri=actor://no-such/thing").await.status, 404);
}

async fn create_is_idempotent_get_returns_state_and_version() {
    let c = cluster();
    let id = format!("create-{}", run_id());
    let counter = Counter { id: id.clone() };
    let first = c.node1.put(&path(&counter), r#"{"state":{}}"#).await;
    assert_eq!(first.status, 201, "{first}");
    assert_eq!(first.json()["created"], json!(true));
    for node in [&c.node2, &c.send_only] {
        let again = node.put(&path(&counter), r#"{"state":{"count":99}}"#).await;
        assert_eq!(again.status, 200, "{node}: {again}");
        assert_eq!(again.json()["created"], json!(false));
    }

    let view = c.send_only.view(&counter).await;
    assert_eq!(view.template, "actor://e2e/counter/{id}");
    assert_eq!(view.params.to_map(), BTreeMap::from([("id".to_owned(), id)]));
    assert_eq!(view.version, 0);
    let state: CounterState = c.send_only.state(&counter).await;
    assert_eq!(state.count, 0, "a repeated create leaves the state untouched");

    let receipt = c.node3.client.send(&counter, step_msg(1)).await.unwrap();
    assert_eq!(c.node3.await_finished(&receipt.id).await.status, MessageState::Done);
    assert_eq!(c.node2.view(&counter).await.version, 1);
    assert_eq!(c.node2.state::<CounterState, _>(&counter).await.count, 1);

    assert_eq!(c.node1.put("/actors/no-such/thing", r#"{"state":{}}"#).await.status, 404);
    assert_eq!(c.node1.get(&format!("/actors/e2e/counter/absent-{}", run_id())).await.status, 404);
}

async fn list_by_prefix_with_paging() {
    let c = cluster();
    let refs: Vec<Recorder> = (1..=5).map(|i| Recorder { id: format!("list-{}-{i}", run_id()) }).collect();
    for (i, r) in refs.iter().enumerate() {
        c.nodes()[i % 4].client.create(r, &RecorderState::default()).await.unwrap();
    }
    c.node1.client.create(&Recorder { id: format!("other-{}", run_id()) }, &RecorderState::default()).await.unwrap();

    let prefix = format!("actor://e2e/recorder/list-{}-", run_id());
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = c.node3.client.list_actors(&prefix, cursor.as_deref(), Some(2)).await.unwrap();
        pages.push(page.items);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [2, 2, 1]);
    let listed: Vec<String> = pages.concat().iter().map(ToString::to_string).collect();
    assert_eq!(listed, refs.iter().map(uri).collect::<Vec<_>>());

    let raw = c.send_only.get(&format!("/actors?prefix=e2e/recorder/list-{}-&limit=10", run_id())).await;
    assert_eq!(raw.status, 200, "{raw}");
    assert_eq!(raw.json()["items"].as_array().map(Vec::len), Some(5));
}

async fn send_through_a_hosting_node_and_through_the_send_only_node_pending_then_done() {
    let c = cluster();
    for via in [&c.node1, &c.send_only] {
        let recorder = Recorder { id: format!("send-{}-{}", run_id(), via.id) };
        via.client.create(&recorder, &RecorderState::default()).await.unwrap();
        let receipt =
            via.client.send_with(&recorder, record("hello"), SendOptions::new().delay(secs(2))).await.unwrap();
        assert!(!receipt.duplicate);
        assert_eq!(via.status(&receipt.id).await.status, MessageState::Pending);
        let pending = via.client.pending(&address(&recorder), None, None).await.unwrap().unwrap();
        assert_eq!(pending.items.iter().map(|m| &m.id).collect::<Vec<_>>(), [&receipt.id]);

        let done = via.await_finished(&receipt.id).await;
        assert_eq!(done.status, MessageState::Done);
        let (finished, due) = (done.finished_at.unwrap(), receipt.deliver_at.unwrap());
        assert!(finished >= due, "finished {finished} before due {due}");
        let records = via.state::<RecorderState, _>(&recorder).await.records;
        let labels: Vec<(&str, &str)> = records.iter().map(|r| (r.label.as_str(), r.message_id.as_str())).collect();
        assert_eq!(labels, [("hello", receipt.id.as_str())]);
        assert!(c.hosting_nodes().iter().any(|n| n.id == records[0].node), "ran on {}", records[0].node);
    }
}

async fn a_delayed_send_is_never_delivered_early() {
    let c = cluster();
    let counter = Counter { id: format!("delay-{}", run_id()) };
    c.node1.client.create(&counter, &CounterState::default()).await.unwrap();
    let sent_at = Utc::now();
    let mut receipts = Vec::new();
    for (i, delay) in [3000, 1000, 2000, 500].into_iter().enumerate() {
        let seq = i as i64 + 1;
        let options = SendOptions::new().delay(millis(delay));
        receipts.push((seq, delay, c.nodes()[i].client.send_with(&counter, step_msg(seq), options).await.unwrap()));
    }
    let at = SendOptions::new().at(sent_at + TimeDelta::milliseconds(2500));
    receipts.push((5, 2500, c.send_only.client.send_with(&counter, step_msg(5), at).await.unwrap()));

    for (seq, delay, receipt) in &receipts {
        // deliverAt is store time; the host clock is the same machine's (1 s slack for VM clock drift).
        let due = receipt.deliver_at.unwrap();
        let requested = TimeDelta::milliseconds(*delay as i64);
        assert!(due >= sent_at + requested - TimeDelta::seconds(1), "seq {seq} due {due}, sent {sent_at}");
    }
    let mut statuses = Vec::new();
    for (seq, _, receipt) in &receipts {
        statuses.push((*seq, c.node2.await_finished(&receipt.id).await));
    }
    let state: CounterState = c.node2.state(&counter).await;
    for (seq, status) in &statuses {
        assert_eq!(status.status, MessageState::Done);
        assert!(status.finished_at.unwrap() >= status.message.deliver_at, "seq {seq} finished before due");
        let entries: Vec<_> = state.entries.iter().filter(|e| e.seq == *seq).collect();
        assert_eq!(entries.len(), 1, "seq {seq}: {entries:?}");
        let due = status.message.deliver_at;
        assert!(entries[0].claimed_at >= due, "seq {seq} claimed at {}, due {due}", entries[0].claimed_at);
    }
    // Delivered in deliverAt order: 500 ms, 1 s, 2 s, 2.5 s (absolute), 3 s.
    assert_eq!(state.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [4, 2, 3, 5, 1]);
}

async fn an_idempotent_resend_with_the_same_id_has_one_outcome() {
    let c = cluster();
    let recorder = Recorder { id: format!("dedup-{}", run_id()) };
    c.node1.client.create(&recorder, &RecorderState::default()).await.unwrap();
    let id = MessageId::random();
    let body = format!(r#"{{"id":"{id}","type":"Record","payload":{{"label":"once","sleepMillis":300}}}}"#);
    let mailbox = format!("{}/messages", path(&recorder));

    let first = c.node1.post(&mailbox, &body).await;
    assert_eq!(first.status, 202, "{first}");
    for node in [&c.node2, &c.send_only, &c.node1] {
        let again = node.post(&mailbox, &body).await;
        assert_eq!(again.status, 200, "{node}: {again}");
        assert_eq!(again.text("/id").as_deref(), Some(id.as_str()));
    }
    assert_eq!(c.send_only.await_finished(&id).await.status, MessageState::Done);
    let afterwards = c.node3.post(&mailbox, &body).await;
    assert_eq!(afterwards.status, 200, "a resend after the outcome is still a duplicate: {afterwards}");

    // The typed client reports the same as a duplicate.
    let options = SendOptions::new().id(id.clone());
    assert!(c.node2.client.send_with(&recorder, record("once"), options).await.unwrap().duplicate);
    assert_eq!(c.node2.state::<RecorderState, _>(&recorder).await.records.len(), 1);
    let mut effects = 0;
    for node in c.hosting_nodes() {
        effects += node.effects(&uri(&recorder)).await.unwrap().len();
    }
    assert_eq!(effects, 1, "the handler ran once");
}

async fn unknown_type_and_bad_payloads() {
    let c = cluster();
    let slow = Slow { id: format!("bad-{}", run_id()) };
    c.node1.client.create(&slow, &SlowState::default()).await.unwrap();
    let mailbox = format!("{}/messages", path(&slow));

    for node in [&c.node1, &c.send_only] {
        let unknown = node.post(&mailbox, r#"{"type":"NoSuchMessage","payload":{}}"#).await;
        assert_eq!(unknown.status, 422, "{node}: {unknown}");
        assert_eq!(unknown.text("/type").as_deref(), Some(problem_types::UNKNOWN_MESSAGE_TYPE));
    }

    let bad_payload = r#"{"type":"Sleep","payload":{"millis":"soon"}}"#;
    let refused = c.node2.post(&mailbox, bad_payload).await;
    assert_eq!(refused.status, 422, "a hosting node decodes at send: {refused}");
    assert_eq!(refused.text("/type").as_deref(), Some(problem_types::INVALID_PAYLOAD));

    // The send-only node cannot decode: it accepts, and the hosting node's delivery applies `Unhandled::Fail`.
    let accepted = c.send_only.post(&mailbox, bad_payload).await;
    assert_eq!(accepted.status, 202, "a send-only node accepts: {accepted}");
    let id = MessageId::new(accepted.text("/id").unwrap()).unwrap();
    let outcome = c.send_only.await_finished(&id).await;
    assert_eq!(outcome.status, MessageState::Failed);
    assert!(outcome.error.is_some());
    let failure = eventually("failure listed", secs(30), async || {
        Ok(c.node3.try_failures(&uri(&slow)).await?.into_iter().find(|f| f.id == id))
    })
    .await;
    assert_eq!(failure.message.message_type, "Sleep");
    assert_eq!(c.node1.view(&slow).await.version, 0, "no state change");
}

async fn a_send_to_an_absent_instance_is_404() {
    let c = cluster();
    for node in [&c.node1, &c.send_only] {
        let path = format!("/actors/e2e/recorder/absent-{}/messages", run_id());
        let response = node.post(&path, r#"{"type":"Record","payload":{"label":"x"}}"#).await;
        assert_eq!(response.status, 404, "{node}: {response}");
        assert_eq!(response.text("/type").as_deref(), Some(problem_types::NO_SUCH_INSTANCE));
    }
}

async fn cancel_a_pending_delayed_message_409_once_finished() {
    let c = cluster();
    let recorder = Recorder { id: format!("cancel-{}", run_id()) };
    c.node1.client.create(&recorder, &RecorderState::default()).await.unwrap();
    let later = SendOptions::new().delay(secs(600));
    let delayed = c.node1.client.send_with(&recorder, record("later"), later).await.unwrap();
    let pending = c.send_only.client.pending(&address(&recorder), None, None).await.unwrap().unwrap();
    assert_eq!(pending.items.iter().map(|m| &m.id).collect::<Vec<_>>(), [&delayed.id]);

    assert_eq!(c.send_only.delete(&format!("/messages/{}", delayed.id)).await.status, 204);
    assert_eq!(c.node2.status(&delayed.id).await.status, MessageState::Cancelled);
    assert!(c.node2.client.pending(&address(&recorder), None, None).await.unwrap().unwrap().items.is_empty());
    let again = c.node3.delete(&format!("/messages/{}", delayed.id)).await;
    assert_eq!(again.status, 409, "{again}");

    let now = c.node2.client.send(&recorder, record("now")).await.unwrap();
    assert_eq!(c.node2.await_finished(&now.id).await.status, MessageState::Done);
    assert_eq!(c.node1.delete(&format!("/messages/{}", now.id)).await.status, 409);
    assert_eq!(c.node1.client.cancel(&now.id).await.unwrap(), CancelOutcome::AlreadyFinished);
    assert_eq!(c.node1.delete(&format!("/messages/{}", MessageId::random())).await.status, 404);
    let labels: Vec<String> =
        c.node3.state::<RecorderState, _>(&recorder).await.records.into_iter().map(|r| r.label).collect();
    assert_eq!(labels, ["now"]);
}

async fn delete_purges_the_mailbox() {
    let c = cluster();
    let recorder = Recorder { id: format!("delete-{}", run_id()) };
    let path = path(&recorder);
    c.node1.client.create(&recorder, &RecorderState::default()).await.unwrap();
    let mut receipts = Vec::new();
    for i in 1..=3 {
        let later = SendOptions::new().delay(secs(600));
        let label = format!("pending-{i}");
        receipts.push(c.nodes()[i].client.send_with(&recorder, record(&label), later).await.unwrap());
    }
    assert_eq!(c.node1.client.pending(&address(&recorder), None, None).await.unwrap().unwrap().items.len(), 3);

    let deleted = c.node2.delete(&path).await;
    assert_eq!(deleted.status, 204, "{deleted}");
    assert_eq!(c.send_only.get(&path).await.status, 404);
    for receipt in &receipts {
        assert_eq!(c.send_only.status(&receipt.id).await.status, MessageState::Purged);
    }
    assert_eq!(c.node3.delete(&path).await.status, 404);
    assert_eq!(c.node3.get(&format!("{path}/messages")).await.status, 404);
    let send = c.node1.post(&format!("{path}/messages"), r#"{"type":"Record","payload":{"label":"x"}}"#).await;
    assert_eq!(send.status, 404);

    // Re-created, it starts empty.
    assert_eq!(c.node1.put(&path, r#"{"state":{}}"#).await.status, 201);
    assert_eq!(c.node1.view(&recorder).await.version, 0);
    assert!(c.node1.client.pending(&address(&recorder), None, None).await.unwrap().unwrap().items.is_empty());
}
