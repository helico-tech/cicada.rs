//! The wire form of the remote turn bodies (spec `remote-turn-http`): every result and report round trips with every
//! field, required nullable fields are required in the decoder and in the schema alike, unknown envelope fields are
//! refused in requests and ignored in responses, and numbers, durations and addresses keep their exact values.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::*;
use crate::retention::Keep;
use crate::retry::Backoff;
use crate::time::parse_instant;

fn addr(text: &str) -> ActorAddress {
    ActorAddress::parse(text).unwrap()
}

fn id(text: &str) -> MessageId {
    MessageId::new(text).unwrap()
}

fn at(text: &str) -> Timestamp {
    parse_instant(text).unwrap()
}

/// A message with every field set, and nothing at its default.
fn full_message() -> Message {
    Message {
        id: id("m1"),
        to: addr("counter/a%20b%2Fc"),
        from: Some(addr("sender/x")),
        message_type: "Add".into(),
        payload: json!({"n": 1, "nested": {"a": [1, 2]}}),
        deliver_at: at("2026-01-01T00:00:00.000001Z"),
        correlation_id: Some("corr".into()),
        headers: BTreeMap::from([("trace".to_owned(), "t1".to_owned())]),
        seq: i64::MAX,
        attempt: 3,
        failures: 2,
        lost_claims: 1,
        last_error: Some("boom".into()),
        retry: Some(RetryPolicy::new(4, Backoff::fixed(Duration::from_secs(5)).unwrap()).unwrap()),
        caused_by: Some(id("m0")),
        retention: Some(Retention { done: Some(Keep::Last { count: 5 }), failed: Some(Keep::Remove) }),
        notification: true,
    }
}

/// A message with every nullable field null and every counter 0.
fn bare_message() -> Message {
    Message {
        id: id("m2"),
        to: addr("counter/b"),
        from: None,
        message_type: "Add".into(),
        payload: json!(null),
        deliver_at: at("2026-01-01T00:00:00Z"),
        correlation_id: None,
        headers: BTreeMap::new(),
        seq: i64::MIN,
        attempt: 0,
        failures: 0,
        lost_claims: 0,
        last_error: None,
        retry: None,
        caused_by: None,
        retention: None,
        notification: false,
    }
}

fn lease(message: Message) -> TurnLease {
    TurnLease {
        actor: message.to.clone(),
        epoch: i64::MIN,
        message,
        state: json!({"count": 7}),
        version: i64::MAX,
        claimed_at: at("2026-01-01T00:00:01Z"),
        lease_until: at("2026-01-01T00:00:31.5Z"),
        ttl: Duration::from_nanos(2),
    }
}

/// Encodes `value` as JSON text and decodes it back as `T`.
fn wire<T: Serialize + DeserializeOwned>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

/// The value at `path` (keys, and array indices as numbers) in `value`.
fn at_path<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(value, |v, k| match k.parse::<usize>() {
        Ok(i) => &v[i],
        Err(_) => &v[*k],
    })
}

fn at_path_mut<'a>(value: &'a mut Value, path: &[&str]) -> &'a mut Value {
    path.iter().fold(value, |v, k| match k.parse::<usize>() {
        Ok(i) => &mut v[i],
        Err(_) => &mut v[*k],
    })
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn full_report() -> TurnReport {
    let mut send = TurnSend::new(addr("other/x"), "Ping", json!({"k": "v"}));
    send.id = id("s1");
    send.delivery = Delivery::After(Duration::from_nanos(2));
    send.correlation_id = Some("c".into());
    send.headers = BTreeMap::from([("h".to_owned(), "1".to_owned())]);
    send.retry = Some(RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap());
    send.retention = Some(Retention { done: Some(Keep::Remove), failed: None });
    let mut at_send = TurnSend::new(addr("other/y"), "Ping", json!({}));
    at_send.id = id("s2");
    at_send.delivery = Delivery::At(at("2027-01-01T00:00:00Z"));
    let mut now_send = TurnSend::new(addr("other/z"), "Ping", json!(null));
    now_send.id = id("s3");
    TurnReport::Returned {
        state: StateReport::Set(json!({"count": 8})),
        effects: TurnEffects {
            watches: vec![
                WatchChange { target: WatchTarget::parse("billing").unwrap(), set: true, existing: false },
                WatchChange { target: WatchTarget::parse("*").unwrap(), set: false, existing: false },
            ],
            creates: vec![
                ActorCreate { address: addr("child/c"), state: json!({}), child: true, pinned_to: None },
                ActorCreate { address: addr("free/f"), state: json!([1]), child: false, pinned_to: None },
            ],
            sends: vec![send, at_send, now_send],
            cancels: vec![id("p1"), id("p2")],
            controls: vec![
                Control { effect: ControlEffect::Delete, actor: addr("a/1") },
                Control { effect: ControlEffect::Pause, actor: addr("a/2") },
                Control { effect: ControlEffect::Resume, actor: addr("a/3") },
                Control { effect: ControlEffect::Purge, actor: addr("a/4") },
            ],
        },
    }
}

/// Scenario: A complete execution lease.
#[test]
fn every_lease_result_round_trips_with_every_field() {
    for result in [
        LeaseTurnsResult::Leased(vec![lease(full_message()), lease(bare_message())]),
        LeaseTurnsResult::Leased(Vec::new()),
        LeaseTurnsResult::ContractMismatch {
            type_name: TypeName::parse("counter").unwrap(),
            registered: Some("sha256:a".into()),
            requested: "sha256:b".into(),
        },
        LeaseTurnsResult::ContractMismatch {
            type_name: TypeName::parse("counter").unwrap(),
            registered: None,
            requested: "sha256:b".into(),
        },
    ] {
        let back: LeaseTurnsResult = wire(&LeaseTurnsResponse::from(result.clone())).into();
        assert_eq!(back, result);
    }
}

#[test]
fn every_renewal_and_completion_result_round_trips() {
    for result in [
        RenewTurnResult::Renewed {
            lease_until: at("2026-01-01T00:00:00.123456Z"),
            ttl: Duration::from_nanos(1_500_000_002),
            cancel_requested: true,
        },
        RenewTurnResult::Renewed {
            lease_until: at("2026-01-01T00:00:00Z"),
            ttl: Duration::ZERO,
            cancel_requested: false,
        },
        RenewTurnResult::LeaseLost,
    ] {
        assert_eq!(RenewTurnResult::from(wire(&RenewTurnResponse::from(result))), result);
    }
    let completion = |outcome: TurnOutcome, cause: Option<FailureCause>| {
        CompleteTurnResult::Completed(TurnCompletion {
            outcome,
            cause,
            version: i64::MIN,
            sent: vec![full_message(), bare_message()],
            cancelled: vec![id("p1")],
        })
    };
    for result in [
        completion(TurnOutcome::Done, None),
        completion(TurnOutcome::Cancelled, None),
        completion(TurnOutcome::Failed("boom".into()), Some(FailureCause::Commit)),
        completion(TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "e".into() }, None),
        completion(
            TurnOutcome::Rescheduled { delivery: Delivery::At(at("2026-01-01T00:00:05Z")), error: "e".into() },
            None,
        ),
        completion(
            TurnOutcome::Rescheduled { delivery: Delivery::After(Duration::from_nanos(2)), error: "e".into() },
            None,
        ),
        CompleteTurnResult::Rejected(TurnFailure {
            cause: FailureCause::Commit,
            retryable: false,
            reason: "No such target".into(),
        }),
        CompleteTurnResult::LeaseLost,
    ] {
        let back = CompleteTurnResult::try_from(wire(&CompleteTurnResponse::from(result.clone()))).unwrap();
        assert_eq!(back, result);
    }
}

/// Scenarios: Returned report, Send effect, Failure and cancellation reports.
#[test]
fn every_report_round_trips_keeping_effect_order_and_delivery_kinds() {
    for report in [
        full_report(),
        TurnReport::Returned { state: StateReport::Unchanged, effects: TurnEffects::default() },
        TurnReport::Returned { state: StateReport::Stop, effects: TurnEffects::default() },
        TurnReport::Returned { state: StateReport::Set(Value::Null), effects: TurnEffects::default() },
        TurnReport::Failed(TurnFailure { cause: FailureCause::Timeout, retryable: true, reason: "slow".into() }),
        TurnReport::Cancelled,
    ] {
        let request = CompleteTurnRequest {
            actor: addr("counter/a%20b%2Fc"),
            epoch: i64::MAX,
            message_id: id("m1"),
            report: report.clone().into(),
        };
        let back = wire(&request);
        assert_eq!(back, request);
        assert_eq!(TurnReport::try_from(back.report).unwrap(), report);
    }
}

#[test]
fn the_wire_form_of_a_lease_and_a_report_is_pinned() {
    let response = LeaseTurnsResponse::from(LeaseTurnsResult::Leased(vec![lease(bare_message())]));
    assert_eq!(
        serde_json::to_string(&response).unwrap(),
        r#"{"result":"leased","leases":[{"actor":"counter/b","epoch":-9223372036854775808,"message":{"id":"m2","to":"counter/b","from":null,"type":"Add","payload":null,"deliverAt":"2026-01-01T00:00:00Z","correlationId":null,"headers":{},"seq":-9223372036854775808,"attempt":0,"failures":0,"lostClaims":0,"lastError":null,"retry":null,"causedBy":null,"retention":null,"notification":false},"state":{"count":7},"version":9223372036854775807,"claimedAt":"2026-01-01T00:00:01Z","leaseUntil":"2026-01-01T00:00:31.500Z","ttl":"PT0.000000002S"}]}"#
    );
    let report = TurnReportView::from(TurnReport::Returned {
        state: StateReport::Unchanged,
        effects: TurnEffects {
            sends: vec![TurnSend { id: id("s"), ..TurnSend::new(addr("a/b"), "T", json!({})) }],
            ..TurnEffects::default()
        },
    });
    assert_eq!(
        serde_json::to_string(&report).unwrap(),
        r#"{"kind":"returned","state":{"kind":"unchanged"},"effects":{"sends":[{"id":"s","to":"a/b","type":"T","payload":{},"headers":{}}],"creates":[],"cancels":[],"watches":[],"controls":[]}}"#
    );
    let completed = CompleteTurnResponse::from(CompleteTurnResult::Completed(TurnCompletion {
        outcome: TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "e".into() },
        cause: None,
        version: 1,
        sent: Vec::new(),
        cancelled: Vec::new(),
    }));
    assert_eq!(
        serde_json::to_string(&completed).unwrap(),
        r#"{"result":"completed","outcome":{"kind":"rescheduled","error":"e"},"cause":null,"version":1,"sent":[],"cancelled":[]}"#
    );
    assert_eq!(serde_json::to_string(&RenewTurnResponse::LeaseLost).unwrap(), r#"{"result":"leaseLost"}"#);
}

// ---- required fields: decoder and schema ----

/// The schema of `T` as JSON, with its definitions.
fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap()
}

/// `schema` with a `$ref` into `root`'s definitions resolved.
fn resolve<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => &root["$defs"][r.trim_start_matches("#/$defs/")],
        None => schema,
    }
}

/// The object schema of `root` (an object, or the variant of a tagged enum whose `tag` is `value`).
fn object_schema<'a>(root: &'a Value, schema: &'a Value, tag: Option<(&str, &str)>) -> &'a Value {
    let schema = resolve(root, schema);
    let Some((tag, value)) = tag else { return schema };
    schema["oneOf"]
        .as_array()
        .expect("a tagged enum")
        .iter()
        .map(|v| resolve(root, v))
        .find(|v| v["properties"][tag]["const"] == value)
        .unwrap_or_else(|| panic!("no variant {value}"))
}

fn required_of(schema: &Value) -> Vec<String> {
    let mut required: Vec<String> = schema["required"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default();
    required.sort();
    required
}

/// Asserts that the schema's required list of the object at `path` in `valid` (through `tag` when it is a variant)
/// is `expected`, and that the decoder agrees: removing a required field fails, removing any other succeeds.
fn assert_required<T: JsonSchema + DeserializeOwned + std::fmt::Debug>(
    valid: Value,
    path: &[&str],
    schema: &Value,
    tag: Option<(&str, &str)>,
    expected: &[&str],
) {
    let root = schema_of::<T>();
    let mut expected: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
    expected.sort();
    assert_eq!(required_of(object_schema(&root, schema, tag)), expected, "schema at {path:?}");
    decode::<T>(valid.clone()).expect("the sample is valid");
    let object = at_path(&valid, path).as_object().unwrap().clone();
    for key in object.keys() {
        let mut changed = valid.clone();
        at_path_mut(&mut changed, path).as_object_mut().unwrap().remove(key);
        let result = decode::<T>(changed);
        let required = expected.contains(key) || tag.is_some_and(|(t, _)| t == key);
        assert_eq!(result.is_err(), required, "removing {key} at {path:?}: {result:?}");
    }
}

/// Scenario: A complete execution lease (every listed field is required, also nullable ones and zero counters); and
/// the schema says the same.
#[test]
fn the_schema_requires_exactly_the_fields_the_decoder_requires() {
    let leased =
        serde_json::to_value(LeaseTurnsResponse::from(LeaseTurnsResult::Leased(vec![lease(bare_message())]))).unwrap();
    let root = schema_of::<LeaseTurnsResponse>();
    let lease_schema = &object_schema(&root, &root, Some(("result", "leased")))["properties"]["leases"]["items"];
    let message_fields = [
        "id",
        "to",
        "from",
        "type",
        "payload",
        "deliverAt",
        "correlationId",
        "headers",
        "seq",
        "attempt",
        "failures",
        "lostClaims",
        "lastError",
        "retry",
        "causedBy",
        "retention",
        "notification",
    ];
    let message_schema = &resolve(&root, lease_schema)["properties"]["message"];
    assert_required::<LeaseTurnsResponse>(
        leased.clone(),
        &["leases", "0", "message"],
        message_schema,
        None,
        &message_fields,
    );
    assert_required::<LeaseTurnsResponse>(
        leased.clone(),
        &["leases", "0"],
        lease_schema,
        None,
        &["actor", "epoch", "message", "state", "version", "claimedAt", "leaseUntil", "ttl"],
    );
    let mismatch = serde_json::to_value(LeaseTurnsResponse::from(LeaseTurnsResult::ContractMismatch {
        type_name: TypeName::parse("counter").unwrap(),
        registered: None,
        requested: "sha256:b".into(),
    }))
    .unwrap();
    assert_required::<LeaseTurnsResponse>(
        mismatch,
        &[],
        &root,
        Some(("result", "contractMismatch")),
        &["result", "type", "registered", "requested"],
    );

    let root = schema_of::<CompleteTurnResponse>();
    let completed = serde_json::to_value(CompleteTurnResponse::from(CompleteTurnResult::Completed(TurnCompletion {
        outcome: TurnOutcome::Rescheduled { delivery: Delivery::After(Duration::from_secs(1)), error: "e".into() },
        cause: None,
        version: 1,
        sent: vec![bare_message()],
        cancelled: Vec::new(),
    })))
    .unwrap();
    assert_required::<CompleteTurnResponse>(
        completed.clone(),
        &[],
        &root,
        Some(("result", "completed")),
        &["result", "outcome", "cause", "version", "sent", "cancelled"],
    );
    let outcome_schema = &object_schema(&root, &root, Some(("result", "completed")))["properties"]["outcome"];
    assert_required::<CompleteTurnResponse>(
        completed,
        &["outcome"],
        outcome_schema,
        Some(("kind", "rescheduled")),
        &["kind", "error"],
    );
    let renewed = serde_json::to_value(RenewTurnResponse::from(RenewTurnResult::Renewed {
        lease_until: at("2026-01-01T00:00:00Z"),
        ttl: Duration::from_secs(30),
        cancel_requested: false,
    }))
    .unwrap();
    let root = schema_of::<RenewTurnResponse>();
    assert_required::<RenewTurnResponse>(
        renewed,
        &[],
        &root,
        Some(("result", "renewed")),
        &["result", "leaseUntil", "ttl", "cancelRequested"],
    );

    // Requests: the send's optional fields and the effect arrays are the documented exceptions.
    let mut send = TurnSend::new(addr("a/b"), "T", json!({}));
    send.delivery = Delivery::After(Duration::from_secs(1));
    send.correlation_id = Some("c".into());
    send.retry = Some(RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap());
    send.retention = Some(Retention { done: Some(Keep::Remove), failed: None });
    let request = serde_json::to_value(CompleteTurnRequest {
        actor: addr("a/b"),
        epoch: 1,
        message_id: id("m"),
        report: TurnReport::Returned {
            state: StateReport::Set(Value::Null),
            effects: TurnEffects {
                sends: vec![send],
                creates: vec![ActorCreate { address: addr("c/d"), state: json!({}), child: false, pinned_to: None }],
                cancels: vec![id("p")],
                watches: vec![WatchChange { target: WatchTarget::parse("*").unwrap(), set: true, existing: false }],
                controls: vec![Control { effect: ControlEffect::Pause, actor: addr("e/f") }],
            },
        }
        .into(),
    })
    .unwrap();
    let root = schema_of::<CompleteTurnRequest>();
    assert_required::<CompleteTurnRequest>(
        request.clone(),
        &[],
        &root,
        None,
        &["actor", "epoch", "messageId", "report"],
    );
    let report_schema = &root["properties"]["report"];
    let effects_schema = &object_schema(&root, report_schema, Some(("kind", "returned")))["properties"]["effects"];
    assert_required::<CompleteTurnRequest>(request.clone(), &["report", "effects"], effects_schema, None, &[]);
    let effects = resolve(&root, effects_schema);
    let item = |name: &str| effects["properties"][name]["items"].clone();
    assert_required::<CompleteTurnRequest>(
        request.clone(),
        &["report", "effects", "sends", "0"],
        &item("sends"),
        None,
        &["id", "to", "type", "payload"],
    );
    assert_required::<CompleteTurnRequest>(
        request.clone(),
        &["report", "effects", "creates", "0"],
        &item("creates"),
        None,
        &["address", "state", "child"],
    );
    assert_required::<CompleteTurnRequest>(
        request.clone(),
        &["report", "effects", "watches", "0"],
        &item("watches"),
        None,
        &["target", "set"],
    );
    assert_required::<CompleteTurnRequest>(
        request.clone(),
        &["report", "effects", "controls", "0"],
        &item("controls"),
        None,
        &["effect", "actor"],
    );
    let state_schema = &object_schema(&root, report_schema, Some(("kind", "returned")))["properties"]["state"];
    assert_required::<CompleteTurnRequest>(
        request,
        &["report", "state"],
        state_schema,
        Some(("kind", "set")),
        &["kind", "value"],
    );
}

#[test]
fn null_is_a_value_of_a_nullable_field_and_not_of_others() {
    let leased =
        serde_json::to_value(LeaseTurnsResponse::from(LeaseTurnsResult::Leased(vec![lease(full_message())]))).unwrap();
    for field in ["from", "correlationId", "lastError", "retry", "causedBy", "retention"] {
        let mut v = leased.clone();
        v["leases"][0]["message"][field] = Value::Null;
        decode::<LeaseTurnsResponse>(v).unwrap_or_else(|e| panic!("{field}: {e}"));
    }
    for field in
        ["id", "to", "type", "deliverAt", "headers", "seq", "attempt", "failures", "lostClaims", "notification"]
    {
        let mut v = leased.clone();
        v["leases"][0]["message"][field] = Value::Null;
        assert!(decode::<LeaseTurnsResponse>(v).is_err(), "{field}");
    }
    // A send's optional fields read null as their default; its headers cannot be null.
    let base = json!({"actor": "a/b", "epoch": 1, "messageId": "m", "report": {"kind": "returned",
        "state": {"kind": "unchanged"}, "effects": {"sends": [{"id": "s", "to": "c/d", "type": "T", "payload": {}}]}}});
    for field in ["deliverAt", "delay", "correlationId", "retry", "retention"] {
        let mut v = base.clone();
        v["report"]["effects"]["sends"][0][field] = Value::Null;
        let request: CompleteTurnRequest = decode(v).unwrap_or_else(|e| panic!("{field}: {e}"));
        let TurnReport::Returned { effects, .. } = TurnReport::try_from(request.report).unwrap() else { panic!() };
        assert_eq!(effects.sends[0], TurnSend { id: id("s"), ..TurnSend::new(addr("c/d"), "T", json!({})) });
    }
    let mut v = base.clone();
    v["report"]["effects"]["sends"][0]["headers"] = Value::Null;
    assert!(decode::<CompleteTurnRequest>(v).is_err());
    for array in ["sends", "creates", "cancels", "watches", "controls"] {
        let mut v = base.clone();
        v["report"]["effects"][array] = Value::Null;
        assert!(decode::<CompleteTurnRequest>(v).is_err(), "{array}");
    }
}

// ---- unknown fields and variants ----

/// Scenario: Required fields and variants (unknown fields at every level of a request).
#[test]
fn a_request_refuses_unknown_fields_at_every_level_but_keeps_state_and_payload_keys_as_data() {
    let base = json!({"actor": "a/b", "epoch": 1, "messageId": "m", "report": {"kind": "returned",
        "state": {"kind": "set", "value": {"anything": 1}},
        "effects": {
            "sends": [{"id": "s", "to": "c/d", "type": "T", "payload": {"free": true},
                "retry": {"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT1S"}},
                "retention": {"done": {"kind": "remove"}}}],
            "creates": [{"address": "e/f", "state": {"free": 1}, "child": true}],
            "watches": [{"target": "*", "set": true}],
            "controls": [{"effect": "pause", "actor": "g/h"}]}}});
    decode::<CompleteTurnRequest>(base.clone()).unwrap();
    for path in [
        &[][..],
        &["report"],
        &["report", "state"],
        &["report", "effects"],
        &["report", "effects", "sends", "0"],
        &["report", "effects", "sends", "0", "retry"],
        &["report", "effects", "sends", "0", "retry", "backoff"],
        &["report", "effects", "sends", "0", "retention"],
        &["report", "effects", "sends", "0", "retention", "done"],
        &["report", "effects", "creates", "0"],
        &["report", "effects", "watches", "0"],
        &["report", "effects", "controls", "0"],
    ] {
        let mut v = base.clone();
        at_path_mut(&mut v, path).as_object_mut().unwrap().insert("extra".into(), json!(1));
        assert!(decode::<CompleteTurnRequest>(v).is_err(), "an extra field at {path:?} was accepted");
    }
    for report in [
        json!({"kind": "cancelled", "extra": 1}),
        json!({"kind": "failed", "cause": "handler",
        "retryable": false, "reason": "r", "extra": 1}),
        json!({"kind": "returned", "state": {"kind": "stop", "x": 1},
        "effects": {}}),
    ] {
        let v = json!({"actor": "a/b", "epoch": 1, "messageId": "m", "report": report});
        assert!(decode::<CompleteTurnRequest>(v.clone()).is_err(), "{v}");
    }
    for (body, extra) in [
        (json!({"owner": "w", "types": {"counter": "sha256:a"}, "limit": 1}), "extra"),
        (json!({"actor": "a/b", "epoch": 1}), "extra"),
    ] {
        let mut v = body.clone();
        v[extra] = json!(1);
        if body.get("owner").is_some() {
            decode::<LeaseTurnsRequest>(body).unwrap();
            assert!(decode::<LeaseTurnsRequest>(v).is_err());
        } else {
            decode::<RenewTurnRequest>(body.clone()).unwrap();
            decode::<ReleaseTurnRequest>(body).unwrap();
            assert!(decode::<RenewTurnRequest>(v.clone()).is_err());
            assert!(decode::<ReleaseTurnRequest>(v).is_err());
        }
    }
    // State and payload keys are data.
    let request: CompleteTurnRequest = decode(base).unwrap();
    let TurnReportView::Returned { state: StateReportView::Set { value }, effects } = request.report else { panic!() };
    assert_eq!(value, json!({"anything": 1}));
    assert_eq!(effects.sends[0].payload, json!({"free": true}));
}

#[test]
fn unknown_variants_fail_in_requests_and_responses() {
    let report = |r: Value| json!({"actor": "a/b", "epoch": 1, "messageId": "m", "report": r});
    for bad in [
        report(json!({"kind": "skipped"})),
        report(json!({"kind": "returned", "state": {"kind": "merge"}, "effects": {}})),
        report(json!({"kind": "failed", "cause": "gremlins", "retryable": false, "reason": "r"})),
        report(json!({"kind": "returned", "state": {"kind": "unchanged"},
            "effects": {"controls": [{"effect": "freeze", "actor": "a/b"}]}})),
    ] {
        assert!(decode::<CompleteTurnRequest>(bad.clone()).is_err(), "{bad}");
    }
    assert!(decode::<LeaseTurnsResponse>(json!({"result": "queued"})).is_err());
    assert!(decode::<RenewTurnResponse>(json!({"result": "extended"})).is_err());
    assert!(decode::<CompleteTurnResponse>(json!({"result": "maybe"})).is_err());
    let completed = |outcome: Value, cause: Value| json!({"result": "completed", "outcome": outcome, "cause": cause, "version": 1, "sent": [], "cancelled": []});
    assert!(decode::<CompleteTurnResponse>(completed(json!({"kind": "skipped"}), Value::Null)).is_err());
    assert!(
        decode::<CompleteTurnResponse>(completed(json!({"kind": "failed", "error": "e"}), json!("gremlins"))).is_err()
    );
    assert!(
        decode::<CompleteTurnResponse>(
            json!({"result": "rejected", "failure": {"cause": "gremlins", "retryable": false,
            "reason": "r"}})
        )
        .is_err()
    );
}

/// Scenario: Strict execution response and tolerant envelope.
#[test]
fn a_response_ignores_unknown_envelope_fields_but_reads_policies_strictly() {
    let mut leased =
        serde_json::to_value(LeaseTurnsResponse::from(LeaseTurnsResult::Leased(vec![lease(full_message())]))).unwrap();
    leased["extra"] = json!(1);
    leased["leases"][0]["extra"] = json!(1);
    leased["leases"][0]["message"]["extra"] = json!(1);
    let back: LeaseTurnsResult = decode::<LeaseTurnsResponse>(leased.clone()).unwrap().into();
    assert_eq!(back, LeaseTurnsResult::Leased(vec![lease(full_message())]));
    for (field, policy) in [
        ("retry", json!({"maxAttempts": 2, "backoff": {"kind": "fibonacci", "delay": "PT1S"}})),
        ("retry", json!({"maxAttempts": 0, "backoff": {"kind": "fixed", "delay": "PT1S"}})),
        ("retry", json!({"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT1S"}, "jitter": true})),
        ("retention", json!({"done": {"kind": "forever"}})),
        ("retention", json!({"done": {"kind": "last", "count": 0}})),
        ("retention", json!({"kept": {"kind": "remove"}})),
    ] {
        let mut v = leased.clone();
        v["leases"][0]["message"][field] = policy.clone();
        assert!(decode::<LeaseTurnsResponse>(v).is_err(), "{field} {policy} read as something");
    }
    let mut renewed = serde_json::to_value(RenewTurnResponse::LeaseLost).unwrap();
    renewed["extra"] = json!({"x": 1});
    assert_eq!(decode::<RenewTurnResponse>(renewed).unwrap(), RenewTurnResponse::LeaseLost);
    let rejected = json!({"result": "rejected", "extra": 1, "failure": {"cause": "commit", "retryable": false,
        "reason": "r", "extra": 2}});
    assert!(matches!(decode::<CompleteTurnResponse>(rejected).unwrap(), CompleteTurnResponse::Rejected { .. }));
}

#[test]
fn contradictory_deliveries_and_causes_fail_conversion() {
    let request = json!({"actor": "a/b", "epoch": 1, "messageId": "m", "report": {"kind": "returned",
        "state": {"kind": "unchanged"}, "effects": {"sends": [{"id": "s", "to": "c/d", "type": "T", "payload": {},
        "deliverAt": "2026-01-01T00:00:00Z", "delay": "PT1S"}]}}});
    let request: CompleteTurnRequest = decode(request).unwrap();
    assert!(TurnReport::try_from(request.report).unwrap_err().to_string().contains("not both"));
    let completed = |outcome: Value, cause: Value| {
        decode::<CompleteTurnResponse>(
            json!({"result": "completed", "outcome": outcome, "cause": cause, "version": 1, "sent": [], "cancelled": []}),
        )
        .unwrap()
    };
    let both = completed(
        json!({"kind": "rescheduled", "error": "e", "deliverAt": "2026-01-01T00:00:00Z", "delay": "PT1S"}),
        Value::Null,
    );
    assert!(CompleteTurnResult::try_from(both).is_err());
    assert!(CompleteTurnResult::try_from(completed(json!({"kind": "done"}), json!("handler"))).is_err());
    assert!(CompleteTurnResult::try_from(completed(json!({"kind": "failed", "error": "e"}), Value::Null)).is_err());
    let null_delivery =
        completed(json!({"kind": "rescheduled", "error": "e", "deliverAt": null, "delay": null}), Value::Null);
    let CompleteTurnResult::Completed(c) = CompleteTurnResult::try_from(null_delivery).unwrap() else { panic!() };
    assert_eq!(c.outcome, TurnOutcome::Rescheduled { delivery: Delivery::Now, error: "e".into() });
}

/// Scenario: Wire and client compatibility (ADR 0052). A returned create carries its pin as `pinnedTo`, an unpinned one
/// leaves it out, absent and null read as none, and a pin beyond the bounds of a claim owner fails conversion.
#[test]
fn a_returned_create_carries_its_pin_and_a_bad_pin_fails_conversion() {
    let pin = NodeId::new("host-a").unwrap();
    let pinned = ActorCreate { address: addr("c/d"), state: json!({}), child: true, pinned_to: Some(pin.clone()) };
    let unpinned = ActorCreate { pinned_to: None, ..pinned.clone() };
    let view =
        TurnEffectsView::from(TurnEffects { creates: vec![pinned.clone(), unpinned.clone()], ..Default::default() });
    assert_eq!(
        serde_json::to_value(&view).unwrap()["creates"],
        json!([{"address": "c/d", "state": {}, "child": true, "pinnedTo": "host-a"},
            {"address": "c/d", "state": {}, "child": true}])
    );
    assert_eq!(TurnEffects::try_from(wire(&view)).unwrap().creates, [pinned, unpinned]);
    for absent in [
        json!({"address": "c/d", "state": {}, "child": false}),
        json!({"address": "c/d", "state": {}, "child": false, "pinnedTo": null}),
    ] {
        assert_eq!(decode::<ActorCreateView>(absent).unwrap().pinned_to, None);
    }
    assert!(decode::<ActorCreateView>(json!({"address": "c/d", "state": {}, "child": false, "pinnedTo": ""})).is_err());
    for bad in ["x".repeat(201), "a\0b".to_owned()] {
        let view = ActorCreateView {
            address: addr("c/d"),
            state: json!({}),
            child: false,
            pinned_to: Some(NodeId::new(bad).unwrap()),
        };
        let effects = TurnEffectsView { creates: vec![view], ..Default::default() };
        assert!(TurnEffects::try_from(effects).unwrap_err().to_string().contains("pinnedTo"));
    }
    let longest = ActorCreateView {
        address: addr("c/d"),
        state: json!({}),
        child: false,
        pinned_to: Some(NodeId::new("é".repeat(200)).unwrap()),
    };
    assert!(ActorCreate::try_from(longest).is_ok(), "200 characters, not bytes");
}

// ---- numbers, durations, addresses ----

/// Scenario: Exact numbers and addresses.
#[test]
fn integers_keep_every_bit_and_refuse_other_number_forms() {
    for epoch in [i64::MIN, -1, 0, i64::MAX] {
        let text = format!(r#"{{"actor":"a/b","epoch":{epoch}}}"#);
        let request: RenewTurnRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(request.epoch, epoch);
        assert_eq!(serde_json::to_string(&request).unwrap(), text);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(decode::<ReleaseTurnRequest>(value).unwrap().epoch, epoch);
    }
    for bad in ["1.0", "1e3", "9223372036854775808", "-9223372036854775809", "\"1\""] {
        let text = format!(r#"{{"actor":"a/b","epoch":{bad}}}"#);
        assert!(serde_json::from_str::<RenewTurnRequest>(&text).is_err(), "{bad}");
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(decode::<RenewTurnRequest>(value).is_err(), "{bad} as a value");
    }
    let leased =
        serde_json::to_value(LeaseTurnsResponse::from(LeaseTurnsResult::Leased(vec![lease(bare_message())]))).unwrap();
    for (field, bad) in [
        ("seq", json!(1.0)),
        ("seq", json!(9223372036854775808u64)),
        ("attempt", json!(-1)),
        ("attempt", json!(4294967296u64)),
        ("failures", json!(1.5)),
        ("lostClaims", json!(1e3)),
    ] {
        let mut v = leased.clone();
        v["leases"][0]["message"][field] = bad.clone();
        assert!(decode::<LeaseTurnsResponse>(v).is_err(), "{field} = {bad}");
    }
    let mut v = leased.clone();
    v["leases"][0]["version"] = json!(1.0);
    assert!(decode::<LeaseTurnsResponse>(v).is_err());
    for bad in [json!(0.5), json!(-1), json!(1e2)] {
        let v = json!({"owner": "w", "types": {"counter": "sha256:a"}, "limit": bad});
        assert!(decode::<LeaseTurnsRequest>(v).is_err());
    }
}

/// Scenario: Duration precision.
#[test]
fn durations_keep_nanoseconds_truncate_below_and_fail_beyond_range() {
    let renewed = |ttl: &str| {
        decode::<RenewTurnResponse>(json!({"result": "renewed", "leaseUntil": "2026-01-01T00:00:00Z", "ttl": ttl,
            "cancelRequested": false}))
    };
    let ttl = |r: RenewTurnResponse| match r {
        RenewTurnResponse::Renewed { ttl, .. } => ttl,
        RenewTurnResponse::LeaseLost => panic!(),
    };
    assert_eq!(ttl(renewed("PT0.000000002S").unwrap()), Duration::from_nanos(2));
    assert_eq!(ttl(renewed("PT0.0000000029S").unwrap()), Duration::from_nanos(2), "truncated below a nanosecond");
    assert_eq!(ttl(renewed("PT1H0.5S").unwrap()), Duration::from_millis(3_600_500));
    assert!(renewed("PT99999999999999999999S").is_err(), "beyond the range of Duration");
    assert!(renewed("-PT1S").is_err());
    let text = serde_json::to_string(&RenewTurnResponse::Renewed {
        lease_until: at("2026-01-01T00:00:00Z"),
        ttl: Duration::from_nanos(2),
        cancel_requested: false,
    })
    .unwrap();
    assert!(text.contains(r#""ttl":"PT0.000000002S""#), "{text}");
    let send = json!({"actor": "a/b", "epoch": 1, "messageId": "m", "report": {"kind": "returned",
        "state": {"kind": "unchanged"}, "effects": {"sends": [{"id": "s", "to": "c/d", "type": "T", "payload": {},
        "delay": "PT0.000000002S"}]}}});
    let request: CompleteTurnRequest = decode(send).unwrap();
    let TurnReport::Returned { effects, .. } = TurnReport::try_from(request.report).unwrap() else { panic!() };
    assert_eq!(effects.sends[0].delivery, Delivery::After(Duration::from_nanos(2)));
}

#[test]
fn addresses_in_bodies_are_canonical_and_not_encoded_again() {
    let request: RenewTurnRequest = decode(json!({"actor": "counter/a%20b%2fc", "epoch": 1})).unwrap();
    assert_eq!(request.actor.as_str(), "counter/a%20b%2Fc");
    assert_eq!(request.actor.name(), "a b/c");
    assert_eq!(serde_json::to_value(&request).unwrap()["actor"], "counter/a%20b%2Fc");
    for bad in ["counter", "counter/a/b", "Counter/a", "counter/%zz"] {
        assert!(decode::<RenewTurnRequest>(json!({"actor": bad, "epoch": 1})).is_err(), "{bad}");
    }
    assert!(decode::<LeaseTurnsRequest>(json!({"owner": "w", "types": {"Bad Name": "h"}, "limit": 1})).is_err());
    assert!(decode::<LeaseTurnsRequest>(json!({"owner": " ", "types": {"counter": "h"}, "limit": 1})).is_err());
}

/// The schema says a required nullable field may be null, as the decoder does, and other fields may not.
#[test]
fn the_schema_allows_null_exactly_for_the_nullable_fields() {
    let allows_null = |root: &Value, schema: &Value| {
        let schema = resolve(root, schema);
        schema["anyOf"].as_array().is_some_and(|a| a.iter().any(|s| s["type"] == "null"))
    };
    let root = schema_of::<TurnMessageView>();
    for (field, schema) in root["properties"].as_object().unwrap() {
        let nullable =
            ["from", "correlationId", "lastError", "retry", "causedBy", "retention"].contains(&field.as_str());
        assert_eq!(allows_null(&root, schema), nullable, "{field}: {schema}");
    }
    let root = schema_of::<LeaseTurnsResponse>();
    let mismatch = object_schema(&root, &root, Some(("result", "contractMismatch")));
    assert!(allows_null(&root, &mismatch["properties"]["registered"]));
    assert!(!allows_null(&root, &mismatch["properties"]["requested"]));
    let root = schema_of::<CompleteTurnResponse>();
    let completed = object_schema(&root, &root, Some(("result", "completed")));
    assert!(allows_null(&root, &completed["properties"]["cause"]));
    assert!(!allows_null(&root, &completed["properties"]["version"]));
}

/// Scenario: Required fields and variants (a record is a JSON object, never its fields as a positional array).
#[test]
fn a_record_given_as_a_positional_array_is_refused_at_every_level() {
    let request = |report: Value| json!({"actor": "a/b", "epoch": 1, "messageId": "m", "report": report});
    let effects =
        |effects: Value| request(json!({"kind": "returned", "state": {"kind": "unchanged"}, "effects": effects}));
    let send = |extra: Value| {
        let mut send = json!({"id": "s", "to": "c/d", "type": "T", "payload": {}});
        send.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        effects(json!({"sends": [send]}))
    };
    decode::<CompleteTurnRequest>(effects(json!({"controls": [{"effect": "delete", "actor": "c/d"}]}))).unwrap();
    for bad in [
        json!(["a/b", 1, "m", {"kind": "cancelled"}]),
        request(json!(["cancelled"])),
        request(json!(["failed", "handler", false, "r"])),
        request(json!({"kind": "returned", "state": ["set", {}], "effects": {}})),
        effects(json!([])),
        effects(json!([[], [], [], [], []])),
        effects(json!({"controls": [["delete", "c/d"]]})),
        effects(json!({"creates": [["c/d", {}, true]]})),
        effects(json!({"watches": [["*", true]]})),
        effects(json!({"sends": [["s", "c/d", "T", {}]]})),
        send(json!({"retry": [2, {"kind": "fixed", "delay": "PT1S"}]})),
        send(json!({"retry": {"maxAttempts": 2, "backoff": ["fixed", "PT1S"]}})),
        send(json!({"retention": {"done": ["remove"]}})),
        send(json!({"retention": [{"kind": "remove"}, null]})),
    ] {
        assert!(decode::<CompleteTurnRequest>(bad.clone()).is_err(), "{bad} was accepted");
    }
    assert!(decode::<LeaseTurnsRequest>(json!(["w", {"counter": "h"}, 1])).is_err());
    assert!(decode::<RenewTurnRequest>(json!(["a/b", 1])).is_err());
    assert!(decode::<ReleaseTurnRequest>(json!(["a/b", 1])).is_err());

    // Responses too: a positional record is no result this version reads.
    let leased =
        serde_json::to_value(LeaseTurnsResponse::from(LeaseTurnsResult::Leased(vec![lease(full_message())]))).unwrap();
    decode::<LeaseTurnsResponse>(leased.clone()).unwrap();
    let positional = |v: &Value| Value::Array(v.as_object().unwrap().values().cloned().collect());
    let mut v = leased.clone();
    v["leases"][0] = positional(&leased["leases"][0]);
    assert!(decode::<LeaseTurnsResponse>(v).is_err());
    let mut v = leased.clone();
    v["leases"][0]["message"] = positional(&leased["leases"][0]["message"]);
    assert!(decode::<LeaseTurnsResponse>(v).is_err());
    let mut v = leased.clone();
    v["leases"][0]["message"]["retry"] = json!([4, {"kind": "fixed", "delay": "PT5S"}]);
    assert!(decode::<LeaseTurnsResponse>(v).is_err());
    assert!(decode::<LeaseTurnsResponse>(json!(["leased", []])).is_err());
    assert!(decode::<RenewTurnResponse>(json!(["leaseLost"])).is_err());
    let completed = |outcome: Value| json!({"result": "completed", "outcome": outcome, "cause": null, "version": 1, "sent": [], "cancelled": []});
    decode::<CompleteTurnResponse>(completed(json!({"kind": "done"}))).unwrap();
    assert!(decode::<CompleteTurnResponse>(completed(json!(["done"]))).is_err());
    assert!(decode::<CompleteTurnResponse>(json!({"result": "rejected", "failure": ["commit", false, "r"]})).is_err());
}

/// The same through `Type::deserialize` called directly, as a caller with a deserializer of its own does: no public
/// decoder of a record skips the object check.
#[test]
fn every_record_called_directly_refuses_a_positional_array_and_takes_its_object() {
    macro_rules! check {
        ($($record:ty: $array:expr, $object:expr;)*) => {$(
            let text = |text: &str| <$record>::deserialize(&mut serde_json::Deserializer::from_str(text));
            assert!(text($array).is_err(), "{} took {}", stringify!($record), $array);
            assert!(text(&$object.to_string()).is_ok(), "{} refused {}", stringify!($record), $object);
            assert!(<$record>::deserialize(serde_json::from_str::<Value>($array).unwrap()).is_err());
            assert!(<$record>::deserialize($object.clone()).is_ok());
        )*};
    }
    let leased =
        serde_json::to_value(LeaseTurnsResponse::from(LeaseTurnsResult::Leased(vec![lease(full_message())]))).unwrap();
    let send = json!({"id": "s", "to": "c/d", "type": "T", "payload": {}});
    let completed = json!({"result": "completed", "outcome": {"kind": "done"}, "cause": null, "version": 1, "sent": [], "cancelled": []});
    check! {
        LeaseTurnsRequest: r#"["w", {"counter": "h"}, 1]"#, json!({"owner": "w", "types": {"counter": "h"}, "limit": 1});
        RenewTurnRequest: r#"["a/b", 1]"#, json!({"actor": "a/b", "epoch": 1});
        ReleaseTurnRequest: r#"["a/b", 1]"#, json!({"actor": "a/b", "epoch": 1});
        CompleteTurnRequest: r#"["a/b", 1, "m", {"kind": "cancelled"}]"#,
            json!({"actor": "a/b", "epoch": 1, "messageId": "m", "report": {"kind": "cancelled"}});
        TurnReportView: r#"["cancelled"]"#, json!({"kind": "cancelled"});
        StateReportView: r#"["set", {}]"#, json!({"kind": "set", "value": {}});
        TurnEffectsView: "[]", json!({});
        TurnSendView: r#"["s", "c/d", "T", {}]"#, send;
        ActorCreateView: r#"["c/d", {}, true]"#, json!({"address": "c/d", "state": {}, "child": true});
        WatchChangeView: r#"["*", true]"#, json!({"target": "*", "set": true});
        ControlView: r#"["delete", "c/d"]"#, json!({"effect": "delete", "actor": "c/d"});
        LeaseTurnsResponse: r#"["leased", []]"#, leased;
        TurnLeaseView: "[]", leased["leases"][0];
        TurnMessageView: "[]", leased["leases"][0]["message"];
        RenewTurnResponse: r#"["leaseLost"]"#, json!({"result": "leaseLost"});
        CompleteTurnResponse: r#"["leaseLost"]"#, completed;
        TurnFailureView: r#"["commit", false, "r"]"#, json!({"cause": "commit", "retryable": false, "reason": "r"});
        TurnOutcomeView: r#"["done"]"#, json!({"kind": "done"});
    }
}
