//! Rules shared by store implementations: the type registry, the creates and the control effects of a commit, claim
//! arguments, envelope JSON, retention of outcomes, the notifications of a watch, cursors and pages.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use serde_json::Value;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use serde_json::{Map, json};

use super::model::{
    ActorCreate, Control, ControlEffect, Delivery, Message, MessageId, OutcomeStatus, OutgoingMessage, Page,
    PageCursor, TypeDef,
};
use super::results::RegisterResult;
use super::{StoreError, StoreResult};
use crate::address::{ActorAddress, TypeName};
use crate::retention::{Keep, Retention};
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use crate::retry::RetryPolicy;
use crate::time::{self, MAX_INSTANT, Timestamp};
use crate::watch::{self, Failed, FailureCause, StopCause, Stopped, Watch, WatchTarget};

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

// ---- creates ----

/// Checks that the creates of one commit agree, per address, on being a child: [`StoreError::Invalid`] otherwise. A
/// commit runs it first, before it reads the store, so the answer does not depend on the lease.
pub fn check_creates(creates: &[ActorCreate]) -> StoreResult<()> {
    let mut child_of = HashMap::new();
    for create in creates {
        if *child_of.entry(&create.address).or_insert(create.child) != create.child {
            return Err(StoreError::Invalid(format!(
                "The creates of {} in one commit disagree on being a child",
                create.address
            )));
        }
    }
    Ok(())
}

/// The reason a commit of `parent` is rejected: its child create of `address` meets an instance that exists with
/// another parent or none.
pub fn foreign_child_reason(address: &ActorAddress, parent: &ActorAddress) -> String {
    format!("Child create of {address}: it exists and is not a child of {parent}")
}

// ---- control effects ----

/// Checks the control effects of a commit of `address` (ADR 0024): [`StoreError::Invalid`] when it deletes `address`
/// itself, or both pauses and resumes one instance, which a turn never asks for. A commit runs it first, before it
/// reads the store, so the answer does not depend on the lease.
pub fn check_controls(address: &ActorAddress, controls: &[Control]) -> StoreResult<()> {
    if controls.iter().any(|c| c.effect == ControlEffect::Delete && c.actor == *address) {
        return Err(StoreError::Invalid(format!("A commit of {address} cannot delete it; a turn stops its own actor")));
    }
    let mut paused = HashMap::new();
    for control in controls {
        let set = match control.effect {
            ControlEffect::Pause => true,
            ControlEffect::Resume => false,
            ControlEffect::Purge | ControlEffect::Delete => continue,
        };
        if *paused.entry(&control.actor).or_insert(set) != set {
            return Err(StoreError::Invalid(format!("A commit both pauses and resumes {}", control.actor)));
        }
    }
    Ok(())
}

/// The reason a commit of `address` is rejected: it deletes `ancestor`, which would remove the commit's own actor.
pub fn ancestor_delete_reason(ancestor: &ActorAddress, address: &ActorAddress) -> String {
    format!("Delete of {ancestor}: it is an ancestor of {address}")
}

/// The pause flag that `controls` set on each instance they pause or resume, which [`check_controls`] keeps to one.
pub fn pauses(controls: &[Control]) -> BTreeMap<&ActorAddress, bool> {
    controls
        .iter()
        .filter_map(|c| match c.effect {
            ControlEffect::Pause => Some((&c.actor, true)),
            ControlEffect::Resume => Some((&c.actor, false)),
            ControlEffect::Purge | ControlEffect::Delete => None,
        })
        .collect()
}

/// The instances that `controls` purge, each once, in address order.
pub fn purges(controls: &[Control]) -> BTreeSet<&ActorAddress> {
    controls.iter().filter(|c| c.effect == ControlEffect::Purge).map(|c| &c.actor).collect()
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
        "lostClaims": message.lost_claims,
        "lastError": message.last_error,
        "retry": message.retry.as_ref().map(RetryPolicy::to_json),
        "causedBy": message.caused_by.as_ref().map(|id| id.as_str()),
        "retention": message.retention.as_ref().map(Retention::to_json),
        "notification": message.notification,
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
        lost_claims: o.get("lostClaims").and_then(Value::as_u64).unwrap_or(0) as u32,
        last_error: str_field("lastError").map(str::to_owned),
        retry: o.get("retry").and_then(|r| RetryPolicy::from_stored(r, &source)),
        caused_by: str_field("causedBy").and_then(|s| MessageId::new(s).ok()),
        retention: o.get("retention").and_then(|r| Retention::from_stored(r, &source)),
        notification: o.get("notification").and_then(Value::as_bool).unwrap_or(false),
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

// ---- watches ----

/// One notification of an event, before the store numbered it: about the actor `about`, which went away or whose
/// message failed, for `watcher`.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    /// The actor of the event; the sender of the notification.
    pub about: ActorAddress,
    /// Who is told.
    pub watcher: ActorAddress,
    message_type: &'static str,
    payload: Value,
    caused_by: Option<MessageId>,
    retention: Option<Retention>,
}

impl Notice {
    /// The [`Failed`] of `message`, which ends `failed` with `error` for `cause`, for `watcher`. It has no `causedBy`,
    /// and it is kept by the `failed` rule of the message's retention for both of its own outcomes: it holds the
    /// payload.
    pub fn failed(message: &Message, error: &str, cause: FailureCause, watcher: &ActorAddress) -> Self {
        let failed = Failed {
            actor: message.to.clone(),
            message_id: message.id.clone(),
            message_type: message.message_type.clone(),
            payload: message.payload.clone(),
            correlation_id: message.correlation_id.clone(),
            from: message.from.clone(),
            error: error.to_owned(),
            attempts: message.attempt,
            cause,
        };
        let rule = message.retention.as_ref().and_then(|r| r.failed.clone());
        Self {
            about: message.to.clone(),
            watcher: watcher.clone(),
            message_type: watch::FAILED,
            payload: serde_json::to_value(failed).expect("a Failed serializes"),
            caused_by: None,
            retention: rule.map(|rule| Retention { done: Some(rule.clone()), failed: Some(rule) }),
        }
    }

    /// The [`Stopped`] of `actor`, which went away for `cause` in the turn of `by` that handled `caused_by` (both
    /// `None` for a delete from outside a turn), for `watcher`.
    pub fn stopped(
        actor: &ActorAddress,
        cause: StopCause,
        by: Option<&ActorAddress>,
        caused_by: Option<&MessageId>,
        watcher: &ActorAddress,
    ) -> Self {
        let stopped = Stopped { actor: actor.clone(), cause, by: by.cloned() };
        Self {
            about: actor.clone(),
            watcher: watcher.clone(),
            message_type: watch::STOPPED,
            payload: serde_json::to_value(stopped).expect("a Stopped serializes"),
            caused_by: caused_by.cloned(),
            retention: None,
        }
    }
}

/// The removals of one event, a stop, a delete or both: what the `Stopped` of each removed actor says.
#[derive(Debug, Clone)]
pub struct Removal<'a> {
    /// The actors whose removal was asked for, with the cause each reads (ADR 0024): [`StopCause::Deleted`] for the
    /// actor of a delete from outside a turn and for the actors a turn deletes, [`StopCause::Stopped`] for the actor
    /// of a turn that stops. Every other removed actor went with an ancestor and reads [`StopCause::Parent`].
    pub roots: Vec<(&'a ActorAddress, StopCause)>,
    /// The actor whose turn removed them; `None` for a delete from outside a turn.
    pub by: Option<&'a ActorAddress>,
    /// The message of that turn; `None` for a delete from outside a turn.
    pub caused_by: Option<&'a MessageId>,
}

impl<'a> Removal<'a> {
    /// The delete of `root` from outside a turn.
    pub fn delete(root: &'a ActorAddress) -> Self {
        Self { roots: vec![(root, StopCause::Deleted)], by: None, caused_by: None }
    }

    /// The removals of a commit of `address` for `message`: the actors of its deletes, and `address` when it stops.
    pub fn of_commit(address: &'a ActorAddress, message: &'a MessageId, controls: &'a [Control], stops: bool) -> Self {
        let deletes =
            controls.iter().filter(|c| c.effect == ControlEffect::Delete).map(|c| (&c.actor, StopCause::Deleted));
        let stop = stops.then_some((address, StopCause::Stopped));
        Self { roots: deletes.chain(stop).collect(), by: Some(address), caused_by: Some(message) }
    }

    /// The `Stopped` of `actor`, one of the removed actors, for `watcher`.
    pub fn notice(&self, actor: &ActorAddress, watcher: &ActorAddress) -> Notice {
        let cause = self.roots.iter().find(|(root, _)| *root == actor).map_or(StopCause::Parent, |(_, cause)| *cause);
        Notice::stopped(actor, cause, self.by, self.caused_by, watcher)
    }
}

/// The messages of the notices of the event `event`: in the order of the actor they are about and then of the
/// watcher, named `<event>.0`, `<event>.1`, … in that order, due now, without headers or retry policy, sent by the
/// actor they are about. A store writes them marked as notifications.
pub fn notifications(event: &MessageId, mut notices: Vec<Notice>) -> Vec<OutgoingMessage> {
    notices.sort_by(|a, b| (&a.about, &a.watcher, a.message_type).cmp(&(&b.about, &b.watcher, b.message_type)));
    notices.dedup_by(|a, b| (&a.about, &a.watcher, a.message_type) == (&b.about, &b.watcher, b.message_type));
    notices
        .into_iter()
        .enumerate()
        .map(|(n, notice)| OutgoingMessage {
            to: notice.watcher,
            message_type: notice.message_type.to_owned(),
            payload: notice.payload,
            id: MessageId::new(format!("{event}.{n}")).expect("an id with a number is not blank"),
            from: Some(notice.about),
            delivery: Delivery::Now,
            correlation_id: None,
            headers: BTreeMap::new(),
            retry: None,
            caused_by: notice.caused_by,
            retention: notice.retention,
        })
        .collect()
}

/// The watch of a stored row; a target or watcher that does not read is corrupt data.
pub fn stored_watch(target: &str, watcher: &str) -> StoreResult<Watch> {
    Ok(Watch {
        target: WatchTarget::parse(target)
            .map_err(|e| StoreError::Corrupt(format!("stored watch target '{target}': {e}")))?,
        watcher: ActorAddress::parse(watcher)
            .map_err(|e| StoreError::Corrupt(format!("stored watcher '{watcher}': {e}")))?,
    })
}

/// The cursor after `watch` in a listing of watches: `target watcher` (neither holds a space).
pub fn watch_cursor(watch: &Watch) -> PageCursor {
    PageCursor(format!("{} {}", watch.target, watch.watcher))
}

/// Decodes a [`watch_cursor`] into `(target, watcher)`.
pub fn decode_watch_cursor(cursor: &PageCursor) -> StoreResult<(String, String)> {
    let (target, watcher) =
        cursor.0.split_once(' ').ok_or_else(|| StoreError::Invalid(format!("Invalid cursor '{cursor}'")))?;
    Ok((target.to_owned(), watcher.to_owned()))
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
            lost_claims: 2,
            last_error: None,
            retry: None,
            caused_by: Some(MessageId::new("m0").unwrap()),
            retention: Some(Retention::REMOVE_ON_DONE),
            notification: true,
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
        assert_eq!((m.failures, m.lost_claims), (0, 0), "an envelope without lostClaims reads as 0");
        assert!(m.retry.is_none() && m.caused_by.is_none() && m.retention.is_none());
    }

    #[test]
    fn the_notifications_of_an_event_are_numbered_in_the_order_of_the_actor_and_then_the_watcher() {
        let a = |text: &str| ActorAddress::parse(text).unwrap();
        let failed = Message { to: a("x/2"), retention: Some(Retention::REMOVE_ON_FAILED), ..message() };
        let root = a("x/2");
        let removal = Removal::delete(&root);
        let notices = vec![
            removal.notice(&a("x/2"), &a("w/b")),
            Notice::failed(&failed, "boom", FailureCause::Timeout, &a("w/b")),
            removal.notice(&a("x/1"), &a("w/b")),
            removal.notice(&a("x/1"), &a("w/a")),
            removal.notice(&a("x/1"), &a("w/a")),
        ];
        let event = MessageId::new("e1").unwrap();
        let written = notifications(&event, notices);
        let order: Vec<(&str, &str, &str, &str)> = written
            .iter()
            .map(|m| (m.id.as_str(), m.from.as_ref().unwrap().as_str(), m.to.as_str(), m.message_type.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("e1.0", "x/1", "w/a", "Stopped"),
                ("e1.1", "x/1", "w/b", "Stopped"),
                ("e1.2", "x/2", "w/b", "Failed"),
                ("e1.3", "x/2", "w/b", "Stopped"),
            ],
            "one per watcher, actor and kind"
        );
        let failure = &written[2];
        assert_eq!(failure.retention, Some(Retention { done: Some(Keep::Remove), failed: Some(Keep::Remove) }));
        assert_eq!((failure.caused_by.as_ref(), failure.payload["messageId"].as_str()), (None, Some("m1")));
        assert_eq!(failure.payload["cause"], "timeout", "the cause it was given");
        assert_eq!(written[3].payload, json!({"actor": "x/2", "cause": "deleted", "by": null}));
        assert!(written.iter().all(|m| m.headers.is_empty() && m.retry.is_none() && m.delivery == Delivery::Now));
    }

    #[test]
    fn watch_cursors() {
        let watch = stored_watch("billing", "w/a").unwrap();
        let cursor = watch_cursor(&watch);
        assert_eq!(cursor.0, "billing w/a");
        assert_eq!(decode_watch_cursor(&cursor).unwrap(), ("billing".to_owned(), "w/a".to_owned()));
        assert!(decode_watch_cursor(&PageCursor("billing".into())).is_err());
        assert!(matches!(stored_watch("a//b", "w/a"), Err(StoreError::Corrupt(_))));
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
    fn creates_of_one_address_must_agree_on_being_a_child() {
        let create = |key: &str, child: bool| ActorCreate {
            address: ActorAddress::parse(&format!("a/{key}")).unwrap(),
            state: json!({}),
            child,
        };
        assert!(check_creates(&[]).is_ok());
        assert!(check_creates(&[create("x", true), create("x", true), create("y", false)]).is_ok());
        let error = check_creates(&[create("y", false), create("x", false), create("x", true)]).unwrap_err();
        assert!(matches!(&error, StoreError::Invalid(m) if m.contains("a/x")), "{error:?}");
    }

    #[test]
    fn a_commit_may_not_delete_its_own_actor_nor_pause_and_resume_one() {
        let a = |text: &str| ActorAddress::parse(text).unwrap();
        let control = |effect, text: &str| Control { effect, actor: a(text) };
        let me = a("a/me");
        let fine = [
            control(ControlEffect::Pause, "a/me"),
            control(ControlEffect::Purge, "a/me"),
            control(ControlEffect::Delete, "a/x"),
            control(ControlEffect::Delete, "a/x"),
            control(ControlEffect::Pause, "a/y"),
            control(ControlEffect::Pause, "a/y"),
            control(ControlEffect::Resume, "a/z"),
        ];
        assert!(check_controls(&me, &fine).is_ok());
        let itself = check_controls(&me, &[control(ControlEffect::Delete, "a/me")]).unwrap_err();
        assert!(matches!(&itself, StoreError::Invalid(m) if m.contains("a/me")), "{itself:?}");
        let both = [
            control(ControlEffect::Resume, "a/y"),
            control(ControlEffect::Purge, "a/y"),
            control(ControlEffect::Pause, "a/y"),
        ];
        let both = check_controls(&me, &both).unwrap_err();
        assert!(matches!(&both, StoreError::Invalid(m) if m.contains("a/y")), "{both:?}");
        assert_eq!(pauses(&fine), BTreeMap::from([(&me, true), (&a("a/y"), true), (&a("a/z"), false)]));
        assert_eq!(purges(&fine), BTreeSet::from([&me]));
    }

    #[test]
    fn the_cause_of_a_removal_depends_on_what_the_commit_names_and_not_on_the_order() {
        let a = |text: &str| ActorAddress::parse(text).unwrap();
        let (me, kid, other, message) = (a("p/me"), a("p/kid"), a("p/other"), MessageId::new("m1").unwrap());
        let controls = [Control { effect: ControlEffect::Delete, actor: kid.clone() }];
        let removal = Removal::of_commit(&me, &message, &controls, true);
        let cause = |actor: &ActorAddress| {
            serde_json::from_value::<Stopped>(removal.notice(actor, &a("w/a")).payload).unwrap().cause
        };
        assert_eq!(
            (cause(&kid), cause(&me), cause(&other)),
            (StopCause::Deleted, StopCause::Stopped, StopCause::Parent)
        );
        let notice = removal.notice(&kid, &a("w/a"));
        assert_eq!(notice.caused_by.as_ref(), Some(&message));
        assert_eq!(notice.payload["by"], "p/me");
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

// ---- test hooks ----

/// A point where a synchronous step of a store stops once, for the tests of close: [`Self::arm`] makes the next
/// [`Self::hit`] report on the returned receiver and block until the returned sender sends (or is dropped).
#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
#[derive(Default)]
pub(crate) struct PausePoint(std::sync::Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>);

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
impl PausePoint {
    /// Arms the point; returns where the hit reports and what lets it go on.
    pub(crate) fn arm(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (reached, reports) = std::sync::mpsc::channel();
        let (go, waits) = std::sync::mpsc::channel();
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some((reached, waits));
        (reports, go)
    }

    /// Stops here when armed, blocking the thread; once only.
    pub(crate) fn hit(&self) {
        let armed = self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some((reached, waits)) = armed {
            let _ = reached.send(());
            let _ = waits.recv();
        }
    }
}
