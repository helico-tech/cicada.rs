//! Rules shared by store implementations: the type registry, claim arguments, envelope JSON, retention of outcomes,
//! cursors and pages.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde_json::Value;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use serde_json::{Map, json};

use super::model::{Message, MessageId, OutcomeStatus, Page, PageCursor, TypeDef};
use super::results::RegisterResult;
use super::{StoreError, StoreResult};
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use crate::address::ActorAddress;
use crate::address::TypeName;
use crate::retention::Keep;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use crate::retention::Retention;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use crate::retry::RetryPolicy;
use crate::time::{self, MAX_INSTANT, Timestamp};

// ---- registry ----

/// The reason of the conflict for `name` registered with a different contract.
pub(crate) fn hash_conflict_reason(name: &TypeName) -> String {
    format!("Type {name} is registered with a different contract")
}

/// The result of registering `def` while its name is already registered as `existing`: registered (as `existing`)
/// for an equal contract hash, a conflict otherwise.
pub fn register_again(existing: TypeDef, def: &TypeDef) -> RegisterResult {
    if existing.contract_hash == def.contract_hash {
        RegisterResult::Registered(existing)
    } else {
        RegisterResult::Conflict { reason: hash_conflict_reason(&def.name), existing }
    }
}

// ---- nodes ----

/// How long a node's row may go without a heartbeat before any node's heartbeat removes it.
pub const NODE_ROW_KEPT: Duration = Duration::from_secs(24 * 3600);

/// The JSON form of a node's hosted types, as stored in the node table.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub fn hosted_to_json(hosted: &BTreeSet<TypeName>) -> Value {
    Value::Array(hosted.iter().map(|t| Value::String(t.as_str().to_owned())).collect())
}

/// Reads the stored JSON form of a node's hosted types; anything else is corrupt data.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub fn hosted_from_json(value: &Value) -> StoreResult<Vec<TypeName>> {
    let names = value.as_array().ok_or_else(|| StoreError::Corrupt("stored hosted types are not a list".to_owned()))?;
    names
        .iter()
        .map(|name| {
            name.as_str()
                .and_then(|n| TypeName::parse(n).ok())
                .ok_or_else(|| StoreError::Corrupt(format!("stored hosted type {name} is invalid")))
        })
        .collect()
}

// ---- claims ----

/// Checks the arguments of `claim_due`: `Ok(false)` when there is nothing to claim (no `types`, or a `limit` of 0),
/// [`StoreError::Invalid`] for a zero `ttl`, `Ok(true)` otherwise.
pub fn check_claim(types: &BTreeSet<TypeName>, ttl: Duration, limit: usize) -> StoreResult<bool> {
    if limit == 0 || types.is_empty() {
        return Ok(false);
    }
    if ttl.is_zero() {
        return Err(StoreError::Invalid("ttl must be positive".into()));
    }
    Ok(true)
}

// ---- envelopes ----

/// The JSON form of a message envelope, as retained with outcomes and served by `/outcomes`; the field names match
/// the HTTP contract.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub fn envelope_to_json(message: &Message) -> Value {
    json!({
        "id": message.id.as_str(),
        "to": message.to.as_str(),
        "from": message.from.as_ref().map(|a| a.as_str()),
        "type": message.message_type,
        "payload": message.payload,
        "deliverAt": time::format_instant(message.deliver_at),
        "correlationId": message.correlation_id,
        "headers": message.headers,
        "seq": message.seq,
        "attempt": message.attempt,
        "failures": message.failures,
        "lastError": message.last_error,
        "retry": message.retry.as_ref().map(RetryPolicy::to_json),
        "causedBy": message.caused_by.as_ref().map(|id| id.as_str()),
        "retention": message.retention.as_ref().map(Retention::to_json),
    })
}

/// Decodes an envelope written by [`envelope_to_json`] (including envelopes
/// retained before later fields existed).
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub fn envelope_from_json(value: &Value) -> StoreResult<Message> {
    let corrupt = |what: &str| StoreError::Corrupt(format!("envelope {what}: {value}"));
    let o = value.as_object().ok_or_else(|| corrupt("is not an object"))?;
    let str_field = |name: &str| o.get(name).and_then(Value::as_str);
    let id = str_field("id").and_then(|s| MessageId::new(s).ok()).ok_or_else(|| corrupt("has no id"))?;
    let source = format!("the envelope of message {id}");
    Ok(Message {
        to: str_field("to").and_then(|s| ActorAddress::parse(s).ok()).ok_or_else(|| corrupt("has no target"))?,
        from: match str_field("from") {
            Some(s) => Some(ActorAddress::parse(s).map_err(|_| corrupt("has a bad sender"))?),
            None => None,
        },
        message_type: str_field("type").ok_or_else(|| corrupt("has no type"))?.to_owned(),
        payload: o.get("payload").cloned().unwrap_or(Value::Null),
        deliver_at: str_field("deliverAt")
            .and_then(|s| time::parse_instant(s).ok())
            .ok_or_else(|| corrupt("has no deliverAt"))?,
        correlation_id: str_field("correlationId").map(str::to_owned),
        headers: o
            .get("headers")
            .and_then(Value::as_object)
            .map(|h| h.iter().map(|(k, v)| (k.clone(), json_text(v))).collect())
            .unwrap_or_default(),
        seq: o.get("seq").and_then(Value::as_i64).ok_or_else(|| corrupt("has no seq"))?,
        attempt: o.get("attempt").and_then(Value::as_u64).unwrap_or(0) as u32,
        failures: o.get("failures").and_then(Value::as_u64).unwrap_or(0) as u32,
        last_error: str_field("lastError").map(str::to_owned),
        retry: o.get("retry").and_then(|r| RetryPolicy::from_stored(r, &source)),
        caused_by: str_field("causedBy").and_then(|s| MessageId::new(s).ok()),
        retention: o.get("retention").and_then(|r| Retention::from_stored(r, &source)),
        id,
    })
}

/// The text of a JSON value: a string's content, or the JSON of anything else.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn json_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Headers as a JSON object.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub fn headers_to_json(headers: &BTreeMap<String, String>) -> Value {
    Value::Object(headers.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect::<Map<_, _>>())
}

/// Headers from a JSON object.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub fn headers_from_json(value: &Value) -> StoreResult<BTreeMap<String, String>> {
    let o = value.as_object().ok_or_else(|| StoreError::Corrupt(format!("headers are not an object: {value}")))?;
    Ok(o.iter().map(|(k, v)| (k.clone(), json_text(v))).collect())
}

// ---- retention ----

/// Until when the outcome of `message`, ending with `status` at `finished_at`, is visible: `finished_at` for
/// [`Keep::Remove`] (never), `finished_at + age` for [`Keep::For`] (at most [`MAX_INSTANT`]), `None` (the store
/// retention) for [`Keep::Last`] and for no rule.
pub fn visible_until(message: &Message, status: OutcomeStatus, finished_at: Timestamp) -> Option<Timestamp> {
    match message.retention.as_ref().and_then(|r| r.rule_for(status)) {
        Some(Keep::Remove) => Some(finished_at),
        Some(Keep::For { age }) => {
            Some(time::add(finished_at, *age).map(|t| t.min(MAX_INSTANT)).unwrap_or(MAX_INSTANT))
        }
        Some(Keep::Last { .. }) | None => None,
    }
}

/// The [`Keep::Last`] count of the outcome of `message` with `status`, or `None` when it does not take part in
/// trimming.
pub fn keep_last(message: &Message, status: OutcomeStatus) -> Option<u32> {
    match message.retention.as_ref().and_then(|r| r.rule_for(status)) {
        Some(Keep::Last { count }) => Some(*count),
        _ => None,
    }
}

/// True when the record of `message` with `status` is removed as soon as it finishes ([`Keep::Remove`]).
pub fn removed_at_once(message: &Message, status: OutcomeStatus) -> bool {
    matches!(message.retention.as_ref().and_then(|r| r.rule_for(status)), Some(Keep::Remove))
}

/// `message` with its content dropped: payload, headers, correlation id and last error.
pub fn stripped(message: &Message) -> Message {
    Message {
        payload: Value::Null,
        headers: BTreeMap::new(),
        correlation_id: None,
        last_error: None,
        ..message.clone()
    }
}

// ---- cursors ----

/// The cursor after `message` in a mailbox listing: `micros:seq`.
pub fn mailbox_cursor(message: &Message) -> PageCursor {
    PageCursor(format!("{}:{}", time::to_micros(message.deliver_at), message.seq))
}

/// Decodes a [`mailbox_cursor`].
pub fn decode_mailbox_cursor(cursor: &PageCursor) -> StoreResult<(i64, i64)> {
    let invalid = || StoreError::Invalid(format!("Invalid cursor '{cursor}'"));
    let (at, seq) = cursor.0.split_once(':').ok_or_else(invalid)?;
    Ok((at.parse().map_err(|_| invalid())?, seq.parse().map_err(|_| invalid())?))
}

/// The cursor after `message` in a listing of every mailbox: `micros:seq:address` (the address last, as it contains colons).
pub fn pending_cursor(message: &Message) -> PageCursor {
    PageCursor(format!("{}:{}:{}", time::to_micros(message.deliver_at), message.seq, message.to))
}

/// Decodes a [`pending_cursor`] into `(address, micros, seq)`.
pub fn decode_pending_cursor(cursor: &PageCursor) -> StoreResult<(String, i64, i64)> {
    let invalid = || StoreError::Invalid(format!("Invalid cursor '{cursor}'"));
    let mut parts = cursor.0.splitn(3, ':');
    let at = parts.next().and_then(|p| p.parse().ok()).ok_or_else(invalid)?;
    let seq = parts.next().and_then(|p| p.parse().ok()).ok_or_else(invalid)?;
    let address = parts.next().ok_or_else(invalid)?.to_owned();
    Ok((address, at, seq))
}

/// The cursor after an outcome in an outcomes listing: `micros:messageId`.
pub fn outcome_cursor(finished_at: Timestamp, id: &MessageId) -> PageCursor {
    PageCursor(format!("{}:{}", time::to_micros(finished_at), id))
}

/// Decodes a [`outcome_cursor`] into `(micros, messageId)`.
pub fn decode_outcome_cursor(cursor: &PageCursor) -> StoreResult<(i64, String)> {
    let invalid = || StoreError::Invalid(format!("Invalid cursor '{cursor}'"));
    let (at, id) = cursor.0.split_once(':').ok_or_else(invalid)?;
    Ok((at.parse().map_err(|_| invalid())?, id.to_owned()))
}

/// Checks a page size.
pub fn check_limit(limit: usize) -> StoreResult<()> {
    if limit == 0 {
        return Err(StoreError::Invalid("limit must be positive".into()));
    }
    Ok(())
}

/// The page of the first `limit` of `items`, which a store reads one beyond the page (at most `limit + 1`): when
/// there are more than `limit`, the page continues after its last item, at `cursor` of that item.
pub fn page_of<T>(mut items: Vec<T>, limit: usize, cursor: impl Fn(&T) -> PageCursor) -> Page<T> {
    if items.len() > limit {
        items.truncate(limit);
        let next = items.last().map(cursor);
        Page::new(items, next)
    } else {
        Page::new(items, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> Message {
        Message {
            id: MessageId::new("m1").unwrap(),
            to: ActorAddress::parse("a/b").unwrap(),
            from: Some(ActorAddress::parse("c/d").unwrap()),
            message_type: "Run".into(),
            payload: json!({"x": 1}),
            deliver_at: "2026-01-01T00:00:00.000001Z".parse().unwrap(),
            correlation_id: Some("c".into()),
            headers: [("h".to_owned(), "v".to_owned())].into(),
            seq: 3,
            attempt: 1,
            failures: 0,
            last_error: None,
            retry: None,
            caused_by: Some(MessageId::new("m0").unwrap()),
            retention: Some(Retention::REMOVE_ON_DONE),
        }
    }

    #[test]
    fn envelope_round_trip() {
        let m = message();
        let json = envelope_to_json(&m);
        assert_eq!(json["deliverAt"], "2026-01-01T00:00:00.000001Z");
        assert_eq!(envelope_from_json(&json).unwrap(), m);
    }

    #[test]
    fn old_envelopes_decode() {
        let json = json!({"id": "m1", "to": "a/b", "from": null, "type": "Run", "payload": {},
            "deliverAt": "2026-01-01T00:00:00Z", "correlationId": null, "headers": {}, "seq": 1, "attempt": 0});
        let m = envelope_from_json(&json).unwrap();
        assert_eq!(m.failures, 0);
        assert!(m.retry.is_none() && m.caused_by.is_none() && m.retention.is_none());
    }

    #[test]
    fn retention_rules() {
        let m = message();
        let t: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        assert_eq!(visible_until(&m, OutcomeStatus::Done, t), Some(t));
        assert!(removed_at_once(&m, OutcomeStatus::Done));
        assert!(!removed_at_once(&m, OutcomeStatus::Failed));
        assert_eq!(visible_until(&m, OutcomeStatus::Failed, t), None);
        let s = stripped(&m);
        assert_eq!(s.payload, Value::Null);
        assert!(s.headers.is_empty() && s.correlation_id.is_none());
    }

    #[test]
    fn registering_again_compares_the_contract_hash() {
        let def = |hash: &str| TypeDef {
            name: TypeName::parse("a").unwrap(),
            contract: json!({}),
            contract_hash: hash.into(),
        };
        assert_eq!(register_again(def("h1"), &def("h1")), RegisterResult::Registered(def("h1")));
        let conflict = register_again(def("h1"), &def("h2"));
        assert!(matches!(conflict, RegisterResult::Conflict { existing, .. } if existing == def("h1")));
    }

    #[test]
    fn claim_arguments() {
        let types = BTreeSet::from([TypeName::parse("a").unwrap()]);
        let ttl = Duration::from_secs(1);
        assert!(check_claim(&types, ttl, 1).unwrap());
        assert!(!check_claim(&types, ttl, 0).unwrap());
        assert!(!check_claim(&BTreeSet::new(), ttl, 1).unwrap());
        assert!(!check_claim(&types, Duration::ZERO, 0).unwrap(), "nothing to claim comes first");
        assert!(matches!(check_claim(&types, Duration::ZERO, 1), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn pages() {
        let cursor = |n: &i32| PageCursor(n.to_string());
        assert_eq!(page_of(vec![1, 2, 3], 2, cursor), Page::new(vec![1, 2], Some(PageCursor("2".into()))));
        assert_eq!(page_of(vec![1, 2], 2, cursor), Page::new(vec![1, 2], None));
        assert_eq!(page_of(Vec::<i32>::new(), 2, cursor), Page::new(vec![], None));
    }

    #[test]
    fn cursors() {
        let m = message();
        assert_eq!(decode_mailbox_cursor(&mailbox_cursor(&m)).unwrap().1, 3);
        let (uri, _, seq) = decode_pending_cursor(&pending_cursor(&m)).unwrap();
        assert_eq!((uri.as_str(), seq), ("a/b", 3));
        assert!(decode_mailbox_cursor(&PageCursor("x".into())).is_err());
    }
}
