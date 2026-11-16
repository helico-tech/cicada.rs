use std::time::Duration;

use serde_json::json;

use super::*;
use crate::retention::Keep;
use crate::retry::Backoff;
use crate::store::{Delivery, Message, MessageStatus, OutcomeRecord, OutcomeStatus};
use crate::time::parse_instant;

fn message() -> Message {
    Message {
        notification: false,
        id: MessageId::new("m1").unwrap(),
        to: ActorAddress::parse("a/b").unwrap(),
        from: None,
        message_type: "Run".into(),
        payload: json!({"x": 1}),
        deliver_at: parse_instant("2026-01-01T00:00:00.000001Z").unwrap(),
        correlation_id: None,
        headers: BTreeMap::new(),
        seq: 3,
        attempt: 0,
        failures: 0,
        lost_claims: 0,
        last_error: None,
        retry: Some(RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(3600)).unwrap()).unwrap()),
        caused_by: None,
        retention: Some(Retention { done: Some(Keep::Last { count: 5 }), failed: None }),
    }
}

#[test]
fn message_view_keeps_its_wire_form() {
    let text = serde_json::to_string(&MessageView::from(message())).unwrap();
    assert_eq!(
        text,
        r#"{"id":"m1","to":"a/b","from":null,"type":"Run","payload":{"x":1},"deliverAt":"2026-01-01T00:00:00.000001Z","correlationId":null,"headers":{},"seq":3,"attempt":0,"failures":0,"lostClaims":0,"lastError":null,"retry":{"maxAttempts":2,"backoff":{"kind":"fixed","delay":"PT1H"}},"causedBy":null,"retention":{"done":{"kind":"last","count":5},"failed":null},"notification":false}"#
    );
    let back: MessageView = serde_json::from_str(&text).unwrap();
    assert_eq!(back, MessageView::from(message()));
    let notification = MessageView::from(Message { notification: true, ..message() });
    assert_eq!(serde_json::to_value(&notification).unwrap()["notification"], true);
    // A view of an earlier node has no mark: it is no notification.
    let earlier = text.replace(r#","notification":false"#, "");
    assert!(!serde_json::from_str::<MessageView>(&earlier).unwrap().notification);
}

/// claim-limit, Scenario: The view of a message (its wire form), and A view from an earlier node.
#[test]
fn message_view_has_the_lost_claims_and_reads_a_view_without_them_as_none() {
    let lost = serde_json::to_value(MessageView::from(Message { lost_claims: 2, ..message() })).unwrap();
    assert_eq!(lost["lostClaims"], 2);
    let text = serde_json::to_string(&MessageView::from(message())).unwrap();
    let earlier = text.replace(r#","lostClaims":0"#, "");
    assert!(!earlier.contains("lostClaims"));
    assert_eq!(serde_json::from_str::<MessageView>(&earlier).unwrap().lost_claims, 0);
}

#[test]
fn status_views_write_every_field() {
    let pending = serde_json::to_value(MessageStatusView::from(MessageStatus::Pending(message()))).unwrap();
    let keys: Vec<&str> = pending.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["id", "status", "error", "failures", "lastError", "finishedAt", "node", "leaseUntil", "message"]);
    assert_eq!(pending["status"], "pending");
    let outcome = OutcomeRecord {
        message_id: MessageId::new("m1").unwrap(),
        actor: ActorAddress::parse("a/b").unwrap(),
        status: OutcomeStatus::Failed,
        error: Some("boom".into()),
        finished_at: parse_instant("2026-01-01T00:00:01Z").unwrap(),
        envelope: message(),
    };
    let view = serde_json::to_value(OutcomeView::from(outcome.clone())).unwrap();
    assert_eq!(view["finishedAt"], "2026-01-01T00:00:01Z");
    assert_eq!(view["status"], "failed");
    let finished = MessageStatusView::from(MessageStatus::Finished(outcome));
    assert!(finished.status.is_finished());
    assert_eq!(finished.error.as_deref(), Some("boom"));
}

#[test]
fn lenient_policies_in_served_envelopes() {
    let mut value = serde_json::to_value(MessageView::from(message())).unwrap();
    value["retry"] = json!({"maxAttempts": 2, "backoff": {"kind": "linear"}});
    value["retention"] = json!("forever");
    let view: MessageView = serde_json::from_value(value).unwrap();
    assert!(view.retry.is_none() && view.retention.is_none());
}

#[test]
fn send_requests_leave_out_absent_fields() {
    let plain = SendRequest::new("Add", json!({"n": 1, "note": null}));
    assert_eq!(serde_json::to_string(&plain).unwrap(), r#"{"type":"Add","payload":{"n":1,"note":null},"headers":{}}"#);
    let full = SendRequest {
        id: Some(MessageId::new("i").unwrap()),
        correlation_id: Some("c".into()),
        ..SendRequest::new("Add", json!({}))
    }
    .with_delivery(Delivery::After(Duration::from_secs(90)));
    assert_eq!(
        serde_json::to_string(&full).unwrap(),
        r#"{"id":"i","type":"Add","payload":{},"delay":"PT1M30S","correlationId":"c","headers":{}}"#
    );
}

fn decode(text: &str) -> Result<SendRequest, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

#[test]
fn send_requests_decode_strictly() {
    let ok = decode(r#"{"type": "Add"}"#).unwrap();
    assert_eq!(ok.payload, json!({}));
    assert_eq!(ok.delivery().unwrap(), Delivery::Now);
    let nulls = decode(
        r#"{"id": null, "type": "Add", "deliverAt": null, "delay": null, "correlationId": null, "retry": null, "retention": null}"#,
    )
    .unwrap();
    assert!(nulls.id.is_none() && nulls.retry.is_none() && nulls.retention.is_none());
    for bad in [
        r#"{"type": "Add", "paylaod": {}}"#,
        r#"{"type": "Add", "retry": {"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT1S"}, "jitter": 1}}"#,
        r#"{"type": "Add", "retry": {"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT1S", "jitter": 1}}}"#,
        r#"{"type": "Add", "retention": {"cancelled": {"kind": "remove"}}}"#,
        r#"{"type": "Add", "retention": {"done": {"kind": "remove", "age": "PT1S"}}}"#,
        r#"{"type": "Add", "id": ""}"#,
        r#"{"payload": {}}"#,
    ] {
        assert!(decode(bad).is_err(), "{bad}");
    }
}

#[test]
fn delays_and_their_errors() {
    let delay = |d: &str| decode(&format!(r#"{{"type": "Add", "delay": {d}}}"#)).map(|r| r.delay.unwrap());
    assert_eq!(delay(r#""PT5S""#).unwrap(), Duration::from_secs(5));
    // ISO-8601 only: no short form, no milliseconds, not negative.
    assert!(delay(r#""5s""#).is_err());
    assert!(delay("1500").is_err());
    assert!(delay("1.5").is_err());
    assert!(delay(r#""-PT1M""#).unwrap_err().contains("negative"));
    let soon = delay(r#""soon""#).unwrap_err();
    assert!(soon.contains("'soon'"), "{soon}");
    let both = decode(r#"{"type": "Add", "delay": "PT1S", "deliverAt": "2030-01-01T00:00:00Z"}"#).unwrap();
    assert!(both.delivery().is_err());
}

#[test]
fn policies_are_refused_or_decoded() {
    for retry in [
        r#"{"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "soon", "max": "PT1M"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "PT1S", "max": 1.5}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": {"s": 1}}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "-PT1S"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT0S"}}"#,
        r#"{"maxAttempts": 0, "backoff": {"kind": "fixed", "delay": "PT1S"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "linear", "delay": "PT1S"}}"#,
    ] {
        assert!(decode(&format!(r#"{{"type": "Add", "retry": {retry}}}"#)).is_err(), "{retry}");
    }
    let defaulted = decode(
        r#"{"type": "Add", "retry": {"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "PT1S", "max": "PT1M"}}}"#,
    )
    .unwrap()
    .retry
    .unwrap();
    assert_eq!(
        defaulted.backoff(),
        &Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(60)).unwrap()
    );
    let policy = decode(
        r#"{"type": "Add", "retention": {"done": {"kind": "last", "count": 10}, "failed": {"kind": "for", "age": "PT1M"}}}"#,
    )
    .unwrap()
    .retention
    .unwrap();
    assert_eq!(
        policy.to_json(),
        json!({"done": {"kind": "last", "count": 10}, "failed": {"kind": "for", "age": "PT1M"}})
    );
}

#[test]
fn problems_and_create_requests() {
    let p = Problem {
        problem_type: problem_types::BAD_REQUEST.into(),
        title: "Bad request".into(),
        status: 400,
        detail: None,
        instance: Some("/actors".into()),
    };
    assert_eq!(
        serde_json::to_string(&p).unwrap(),
        r#"{"type":"urn:cicada:problem:bad-request","title":"Bad request","status":400,"detail":null,"instance":"/actors"}"#
    );
    assert_eq!(
        serde_json::to_string(&CreateActorRequest { state: json!({}), pinned_to: None }).unwrap(),
        r#"{"state":{}}"#
    );
    assert!(serde_json::from_str::<CreateActorRequest>(r#"{"stat": 1}"#).is_err());
    assert!(
        serde_json::from_str::<CreateActorRequest>("{}").unwrap_err().to_string().contains("missing field `state`")
    );
    // Null decodes, so that the node can refuse it as an invalid state (422) rather than a malformed body.
    assert_eq!(serde_json::from_str::<CreateActorRequest>(r#"{"state": null}"#).unwrap().state, Value::Null);
    // A pin (ADR 0052): written when set, absent or null for none, never blank.
    let pinned = CreateActorRequest { state: json!({}), pinned_to: Some(NodeId::new("host-a").unwrap()) };
    assert_eq!(serde_json::to_string(&pinned).unwrap(), r#"{"state":{},"pinnedTo":"host-a"}"#);
    let read = |text: &str| serde_json::from_str::<CreateActorRequest>(text).map(|r| r.pinned_to);
    assert_eq!(read(r#"{"state":{},"pinnedTo":"host-a"}"#).unwrap(), pinned.pinned_to);
    assert_eq!(read(r#"{"state":{},"pinnedTo":null}"#).unwrap(), None);
    assert!(read(r#"{"state":{},"pinnedTo":" "}"#).is_err());
}

/// An actor view writes its pin, `null` for none, and reads a view of an older server without one as none.
#[test]
fn an_actor_view_writes_its_pin_and_reads_an_older_view_without_one() {
    let older = json!({"address": "a/b", "type": "a", "state": {}, "version": 0,
        "createdAt": "2026-01-01T00:00:00Z", "paused": false, "parent": null});
    let view: ActorView = serde_json::from_value(older).unwrap();
    assert_eq!(view.pinned_to, None);
    assert_eq!(serde_json::to_value(&view).unwrap()["pinnedTo"], Value::Null, "written as null");
    let pinned = ActorView { pinned_to: Some(NodeId::new("host-a").unwrap()), ..view };
    assert_eq!(serde_json::to_value(&pinned).unwrap()["pinnedTo"], "host-a");
}

/// type-registry, Scenarios: A registration request, A malformed request and Client-supplied identity fields.
#[test]
fn register_type_requests_are_strict_in_the_envelope_and_keep_schema_keywords() {
    let request = RegisterTypeRequest {
        state: json!({"type": "object"}),
        messages: BTreeMap::from([("Add".to_owned(), json!(true))]),
    };
    let text = serde_json::to_string(&request).unwrap();
    assert_eq!(text, r#"{"state":{"type":"object"},"messages":{"Add":true}}"#);
    assert_eq!(serde_json::from_str::<RegisterTypeRequest>(&text).unwrap(), request);
    for field in ["name", "type", "hash"] {
        let body = format!(r#"{{"state": {{}}, "messages": {{}}, "{field}": "x"}}"#);
        let error = serde_json::from_str::<RegisterTypeRequest>(&body).unwrap_err().to_string();
        assert!(error.contains("unknown field"), "{error}");
    }
    for missing in [r#"{"state": {}}"#, r#"{"messages": {}}"#, r#"{"state": {}, "messages": []}"#] {
        assert!(serde_json::from_str::<RegisterTypeRequest>(missing).is_err(), "{missing}");
    }
    let kept = serde_json::from_str::<RegisterTypeRequest>(r#"{"state": {"x-own": 1}, "messages": {}}"#).unwrap();
    assert_eq!(kept.state, json!({"x-own": 1}));
    // Null decodes, so that the node can refuse it as an invalid contract (422) rather than a malformed body.
    let null = serde_json::from_str::<RegisterTypeRequest>(r#"{"state": null, "messages": {}}"#).unwrap();
    assert_eq!(null.state, Value::Null);
}
