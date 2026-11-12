//! Watches (ADR 0018, ADR 0023): an actor, the *watcher*, is told with a message when a message to a watched actor
//! ends `failed`, and when a watched actor goes away.
//!
//! A watch is a stored pair of a [`WatchTarget`] and a watcher. The store writes the *notification*, a [`Failed`] or a
//! [`Stopped`], into the watcher's mailbox in the transaction of the event. A watcher takes both as variants of its own
//! message family, under exactly these type names and with these types, and handles them in its one handler:
//!
//! ```
//! use cicada::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Serialize, Deserialize, JsonSchema)]
//! #[serde(tag = "type")]
//! enum SupervisorMsg {
//!     Failed(cicada::Failed),
//!     Stopped(cicada::Stopped),
//!     Restart { name: String },
//! }
//! ```
//!
//! A watch on one actor tells of its failures and of its going away; a watch on a type prefix or on every type tells of
//! failures only. A notification that fails notifies nobody, so two actors that watch each other cannot loop.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::address::{ActorAddress, ActorRef, TypeName};
use crate::contract::{self, Contract};
use crate::store::MessageId;

/// The notification of a message to a watched actor that ended `failed`: what the failed message was and why it
/// failed. Its type name is `Failed`. A watcher's message family declares it as `Failed(cicada::Failed)`.
///
/// Its schema is part of every watcher's contract; a snapshot test pins it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Failed {
    /// The actor the failed message was for: the actor of the event, not the target of the watch.
    pub actor: ActorAddress,
    /// The id of the failed message.
    pub message_id: MessageId,
    /// The type name of the failed message.
    pub message_type: String,
    /// The payload of the failed message.
    pub payload: Value,
    /// The correlation id of the failed message; null when it has none.
    pub correlation_id: Option<String>,
    /// The sender of the failed message; null for a message sent from outside a turn.
    pub from: Option<ActorAddress>,
    /// The error it failed with.
    pub error: String,
    /// How often the message was claimed: its `attempt`.
    pub attempts: u32,
    /// What failed, one of:
    /// - `handler`: the handler returned an error or panicked and no retry is left, or it returned a state that cannot
    ///   be stored;
    /// - `node`: a read of the store the node needed failed, also one the handler passed on, or a panic outside the
    ///   handler;
    /// - `refused`: the handler refused the message for good;
    /// - `timeout`: the turn ran past its maximum duration;
    /// - `message`: the message cannot be decoded;
    /// - `state`: the state or the address of the actor cannot be read by its type;
    /// - `commit`: the effects of the turn were refused, or its commit was rejected or refused;
    /// - `claim-limit`: the message reached the claim limit.
    pub cause: FailureCause,
}

/// Why a message ended `failed` (ADR 0028). The node decides it where it decides that the message fails. Written in
/// kebab-case, in place in the schema of [`Failed`]. A closed set: a value added later changes the contract of every
/// type that watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(inline)]
pub enum FailureCause {
    /// The handler returned an error or panicked, and no retry is left; or it returned a new state that cannot be
    /// stored.
    Handler,
    /// The node or the store failed around the turn: a read the node needed failed, also one the handler passed on,
    /// or a panic outside the handler.
    Node,
    /// The handler refused the message for good (a permanent failure).
    Refused,
    /// The turn ran past its maximum duration.
    Timeout,
    /// The message cannot be decoded.
    Message,
    /// The state or the address of the actor cannot be read by its type.
    State,
    /// The effects of the turn were refused, or its commit was rejected or refused.
    Commit,
    /// The message reached the claim limit (ADR 0025).
    ClaimLimit,
}

impl FailureCause {
    /// The name on the wire: `handler`, `node`, `refused`, `timeout`, `message`, `state`, `commit` or `claim-limit`.
    pub fn name(self) -> &'static str {
        match self {
            FailureCause::Handler => "handler",
            FailureCause::Node => "node",
            FailureCause::Refused => "refused",
            FailureCause::Timeout => "timeout",
            FailureCause::Message => "message",
            FailureCause::State => "state",
            FailureCause::Commit => "commit",
            FailureCause::ClaimLimit => "claim-limit",
        }
    }
}

impl fmt::Display for FailureCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The notification of a watched actor that went away. Its type name is `Stopped`. A watcher's message family declares
/// it as `Stopped(cicada::Stopped)`. Only a watch on the actor's own address tells of it.
///
/// Its schema is part of every watcher's contract; a snapshot test pins it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    /// The actor that went away.
    pub actor: ActorAddress,
    /// Why it went away.
    pub cause: StopCause,
    /// The actor whose turn removed it: the actor itself when it stopped, the stopping ancestor when it went with it;
    /// null for a removal from outside a turn.
    pub by: Option<ActorAddress>,
}

/// Why an actor went away. Written as `stopped`, `deleted` or `parent`, in place in the schema of [`Stopped`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum StopCause {
    /// Its own turn stopped it.
    Stopped,
    /// It was deleted from outside a turn.
    Deleted,
    /// It was removed with its parent (ADR 0019).
    Parent,
}

/// The two notifications as a message family: the schemas a watcher's contract must hold.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum Notification {
    Failed(Failed),
    Stopped(Stopped),
}

/// The type name of [`Failed`].
pub const FAILED: &str = "Failed";
/// The type name of [`Stopped`].
pub const STOPPED: &str = "Stopped";

/// The message schemas of [`Failed`] and [`Stopped`], as a contract holds them.
pub(crate) fn notification_schemas() -> &'static BTreeMap<String, Value> {
    static SCHEMAS: LazyLock<BTreeMap<String, Value>> = LazyLock::new(|| {
        crate::schema::message_schemas::<Notification>().expect("the notifications are a message family")
    });
    &SCHEMAS
}

/// Why the type of `contract` may not watch, or `None` when it may: its contract must hold the message types `Failed`
/// and `Stopped` with the schemas of [`Failed`] and [`Stopped`], compared as a contract hash compares schemas (their
/// descriptions left out).
pub(crate) fn watcher_problem(contract: &Contract) -> Option<String> {
    for (name, expected) in notification_schemas() {
        let problem = match contract.message_schemas().get(name) {
            None => format!("has no message type {name}"),
            Some(schema) if !contract::same_schema(schema, expected) => {
                format!("has a message type {name} that is not cicada::{name}")
            }
            Some(_) => continue,
        };
        return Some(format!(
            "{} cannot watch: its message family {problem}; a watcher declares Failed(cicada::Failed) and \
             Stopped(cicada::Stopped)",
            contract.type_name()
        ));
    }
    None
}

/// A text that is not a watch target. It prints why, e.g. "Invalid watch target 'a//b': …".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct WatchTargetError(String);

/// What a watch watches: one actor, the types under a prefix, or every type.
///
/// As text, over HTTP and on the command line: an address (`billing.invoices/acme`), a type prefix (`billing`) or `*`.
/// A prefix ends at a dot: `billing` matches the types `billing` and `billing.invoices`, not `billings`. A target need
/// not exist: a watch survives the delete and re-create of its target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WatchTarget {
    /// One actor: told of its failures and of its going away.
    Actor(ActorAddress),
    /// The types named by the prefix, at a segment boundary: told of their failures only.
    Types(TypeName),
    /// Every type, written `*`: told of every failure.
    Every,
}

impl WatchTarget {
    /// The text of [`WatchTarget::Every`].
    pub const EVERY: &'static str = "*";

    /// Reads a target from its text: an address when it holds a `/`, `*`, or else a type name. Anything else fails.
    pub fn parse(text: &str) -> Result<Self, WatchTargetError> {
        let invalid = |e: &dyn fmt::Display| {
            WatchTargetError(format!(
                "Invalid watch target '{text}': {e}; a target is an address (type/key), a type prefix or *"
            ))
        };
        if text.contains('/') {
            ActorAddress::parse(text).map(WatchTarget::Actor).map_err(|e| invalid(&e))
        } else if text == Self::EVERY {
            Ok(WatchTarget::Every)
        } else {
            TypeName::parse(text).map(WatchTarget::Types).map_err(|e| invalid(&e))
        }
    }

    /// The text form: the address, the type prefix, or `*`.
    pub fn as_str(&self) -> &str {
        match self {
            WatchTarget::Actor(address) => address.as_str(),
            WatchTarget::Types(prefix) => prefix.as_str(),
            WatchTarget::Every => Self::EVERY,
        }
    }

    /// True when the failure of a message to `actor` matches this target: its address, a prefix of its type name at a
    /// dot, or every type.
    pub fn matches_failure(&self, actor: &ActorAddress) -> bool {
        match self {
            WatchTarget::Actor(address) => address == actor,
            WatchTarget::Types(prefix) => {
                let name = actor.type_name().as_str();
                name.strip_prefix(prefix.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            }
            WatchTarget::Every => true,
        }
    }

    /// True when the going away of `actor` matches this target: only its own address does.
    pub fn matches_removal(&self, actor: &ActorAddress) -> bool {
        matches!(self, WatchTarget::Actor(address) if address == actor)
    }
}

/// The texts of every target that the failure of a message to `actor` matches: its address, each prefix of its type
/// name at a dot, longest first, and `*`. A store looks the watches of a failure up by these.
#[cfg_attr(not(any(feature = "sqlite", feature = "postgres", feature = "testing")), allow(dead_code))]
pub(crate) fn failure_targets(actor: &ActorAddress) -> Vec<String> {
    let name = actor.type_name().as_str();
    let mut targets = vec![actor.as_str().to_owned()];
    targets.extend(name.match_indices('.').map(|(at, _)| name[..at].to_owned()).rev());
    targets.insert(1, name.to_owned());
    targets.push(WatchTarget::EVERY.to_owned());
    targets
}

impl fmt::Display for WatchTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WatchTarget {
    type Err = WatchTargetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl From<ActorAddress> for WatchTarget {
    fn from(address: ActorAddress) -> Self {
        WatchTarget::Actor(address)
    }
}

impl Serialize for WatchTarget {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WatchTarget {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for WatchTarget {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "WatchTarget".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "An address (type/key), a type prefix that ends at a dot, or * for every type."
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

/// What [`Turn::watch`](crate::Turn::watch) and [`Turn::unwatch`](crate::Turn::unwatch) take: a [`WatchTarget`], a
/// typed ref, an address, or a target as text, which is read when the call is made.
pub trait IntoWatchTarget {
    /// The target; fails for a text that is no target, or a ref that has no address.
    fn into_watch_target(self) -> Result<WatchTarget, WatchTargetError>;
}

impl IntoWatchTarget for WatchTarget {
    fn into_watch_target(self) -> Result<WatchTarget, WatchTargetError> {
        Ok(self)
    }
}

impl IntoWatchTarget for &WatchTarget {
    fn into_watch_target(self) -> Result<WatchTarget, WatchTargetError> {
        Ok(self.clone())
    }
}

impl IntoWatchTarget for ActorAddress {
    fn into_watch_target(self) -> Result<WatchTarget, WatchTargetError> {
        Ok(WatchTarget::Actor(self))
    }
}

impl IntoWatchTarget for &ActorAddress {
    fn into_watch_target(self) -> Result<WatchTarget, WatchTargetError> {
        Ok(WatchTarget::Actor(self.clone()))
    }
}

impl IntoWatchTarget for &str {
    fn into_watch_target(self) -> Result<WatchTarget, WatchTargetError> {
        WatchTarget::parse(self)
    }
}

impl<R: ActorRef> IntoWatchTarget for &R {
    fn into_watch_target(self) -> Result<WatchTarget, WatchTargetError> {
        self.address().map(WatchTarget::Actor).map_err(|e| WatchTargetError(e.to_string()))
    }
}

/// One stored watch: `watcher` is told of the events of `target`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Watch {
    /// What it watches.
    pub target: WatchTarget,
    /// Who is told.
    pub watcher: ActorAddress,
}

/// A change of a watch buffered by a turn and applied by its commit, before its creates and sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchChange {
    /// The target; the watcher is the committing actor.
    pub target: WatchTarget,
    /// True to set the watch, false to remove it.
    pub set: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn address(text: &str) -> ActorAddress {
        ActorAddress::parse(text).unwrap()
    }

    fn target(text: &str) -> WatchTarget {
        WatchTarget::parse(text).unwrap()
    }

    /// The JSON Schemas of the two notifications as a contract holds them. A change here changes the contract of every
    /// watcher, so it must be deliberate.
    #[test]
    fn the_schemas_of_the_notifications_are_pinned() {
        let schemas = notification_schemas();
        assert_eq!(schemas.keys().collect::<Vec<_>>(), [FAILED, STOPPED]);
        let failed = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Failed",
            "description": "The notification of a message to a watched actor that ended `failed`: what the failed \
                            message was and why it\nfailed. Its type name is `Failed`. A watcher's message family \
                            declares it as `Failed(cicada::Failed)`.\n\nIts schema is part of every watcher's \
                            contract; a snapshot test pins it.",
            "type": "object",
            "properties": {
                "actor": {
                    "description": "The actor the failed message was for: the actor of the event, not the target of \
                                    the watch.",
                    "type": "string",
                    "pattern": "^[a-z][a-z0-9._-]*/[^/]+$"
                },
                "messageId": {"description": "The id of the failed message.", "type": "string", "minLength": 1},
                "messageType": {"description": "The type name of the failed message.", "type": "string"},
                "payload": {"description": "The payload of the failed message."},
                "correlationId": {
                    "description": "The correlation id of the failed message; null when it has none.",
                    "type": ["string", "null"]
                },
                "from": {
                    "description": "The sender of the failed message; null for a message sent from outside a turn.",
                    "type": ["string", "null"],
                    "pattern": "^[a-z][a-z0-9._-]*/[^/]+$"
                },
                "error": {"description": "The error it failed with.", "type": "string"},
                "attempts": {
                    "description": "How often the message was claimed: its `attempt`.",
                    "type": "integer",
                    "format": "uint32",
                    "minimum": 0
                },
                "cause": {
                    "description": concat!(
                        "What failed, one of:\n",
                        "- `handler`: the handler returned an error or panicked and no retry is left, or it returned ",
                        "a state that cannot\n  be stored;\n",
                        "- `node`: a read of the store the node needed failed, also one the handler passed on, or a ",
                        "panic outside the\n  handler;\n",
                        "- `refused`: the handler refused the message for good;\n",
                        "- `timeout`: the turn ran past its maximum duration;\n",
                        "- `message`: the message cannot be decoded;\n",
                        "- `state`: the state or the address of the actor cannot be read by its type;\n",
                        "- `commit`: the effects of the turn were refused, or its commit was rejected or refused;\n",
                        "- `claim-limit`: the message reached the claim limit."
                    ),
                    "oneOf": [
                        {
                            "description": "The handler returned an error or panicked, and no retry is left; or it \
                                            returned a new state that cannot be\nstored.",
                            "type": "string",
                            "const": "handler"
                        },
                        {
                            "description": "The node or the store failed around the turn: a read the node needed \
                                            failed, also one the handler passed on,\nor a panic outside the handler.",
                            "type": "string",
                            "const": "node"
                        },
                        {
                            "description": "The handler refused the message for good (a permanent failure).",
                            "type": "string",
                            "const": "refused"
                        },
                        {"description": "The turn ran past its maximum duration.", "type": "string", "const": "timeout"},
                        {"description": "The message cannot be decoded.", "type": "string", "const": "message"},
                        {
                            "description": "The state or the address of the actor cannot be read by its type.",
                            "type": "string",
                            "const": "state"
                        },
                        {
                            "description": "The effects of the turn were refused, or its commit was rejected or \
                                            refused.",
                            "type": "string",
                            "const": "commit"
                        },
                        {
                            "description": "The message reached the claim limit (ADR 0025).",
                            "type": "string",
                            "const": "claim-limit"
                        }
                    ]
                }
            },
            "required": ["actor", "messageId", "messageType", "payload", "error", "attempts", "cause"]
        });
        let stopped = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Stopped",
            "description": "The notification of a watched actor that went away. Its type name is `Stopped`. A \
                            watcher's message family declares\nit as `Stopped(cicada::Stopped)`. Only a watch on the \
                            actor's own address tells of it.\n\nIts schema is part of every watcher's contract; a \
                            snapshot test pins it.",
            "type": "object",
            "properties": {
                "actor": {
                    "description": "The actor that went away.",
                    "type": "string",
                    "pattern": "^[a-z][a-z0-9._-]*/[^/]+$"
                },
                "cause": {
                    "description": "Why it went away.",
                    "oneOf": [
                        {"description": "Its own turn stopped it.", "type": "string", "const": "stopped"},
                        {"description": "It was deleted from outside a turn.", "type": "string", "const": "deleted"},
                        {
                            "description": "It was removed with its parent (ADR 0019).",
                            "type": "string",
                            "const": "parent"
                        }
                    ]
                },
                "by": {
                    "description": "The actor whose turn removed it: the actor itself when it stopped, the stopping \
                                    ancestor when it went with it;\nnull for a removal from outside a turn.",
                    "type": ["string", "null"],
                    "pattern": "^[a-z][a-z0-9._-]*/[^/]+$"
                }
            },
            "required": ["actor", "cause"]
        });
        assert_eq!(schemas[FAILED], failed, "{}", serde_json::to_string_pretty(&schemas[FAILED]).unwrap());
        assert_eq!(schemas[STOPPED], stopped, "{}", serde_json::to_string_pretty(&schemas[STOPPED]).unwrap());
    }

    #[test]
    fn a_failed_is_written_with_every_field_and_read_back() {
        let full = Failed {
            actor: address("billing.invoices/acme"),
            message_id: MessageId::new("m1").unwrap(),
            message_type: "Charge".into(),
            payload: json!({"amount": 3}),
            correlation_id: Some("run-7".into()),
            from: Some(address("billing/ledger")),
            error: "declined".into(),
            attempts: 2,
            cause: FailureCause::Refused,
        };
        let written = serde_json::to_value(&full).unwrap();
        assert_eq!(
            written,
            json!({"actor": "billing.invoices/acme", "messageId": "m1", "messageType": "Charge",
                "payload": {"amount": 3}, "correlationId": "run-7", "from": "billing/ledger", "error": "declined",
                "attempts": 2, "cause": "refused"})
        );
        assert_eq!(serde_json::from_value::<Failed>(written).unwrap(), full);
        let bare = Failed { correlation_id: None, from: None, ..full };
        let written = serde_json::to_value(&bare).unwrap();
        assert_eq!(written["correlationId"], Value::Null, "always written");
        assert_eq!(written["from"], Value::Null, "always written");
        assert_eq!(serde_json::from_value::<Failed>(written).unwrap(), bare);
    }

    #[test]
    fn every_cause_is_written_in_kebab_case_and_read_back() {
        let causes = [
            (FailureCause::Handler, "handler"),
            (FailureCause::Node, "node"),
            (FailureCause::Refused, "refused"),
            (FailureCause::Timeout, "timeout"),
            (FailureCause::Message, "message"),
            (FailureCause::State, "state"),
            (FailureCause::Commit, "commit"),
            (FailureCause::ClaimLimit, "claim-limit"),
        ];
        for (cause, text) in causes {
            assert_eq!(serde_json::to_value(cause).unwrap(), json!(text));
            assert_eq!(serde_json::from_value::<FailureCause>(json!(text)).unwrap(), cause);
            assert_eq!((cause.name(), cause.to_string()), (text, text.to_owned()));
        }
        assert!(serde_json::from_value::<FailureCause>(json!("claim_limit")).is_err(), "a closed set");
    }

    /// A `Failed` of the release before 0.14.0 has no cause: a watcher of this release cannot decode it.
    #[test]
    fn a_failed_without_a_cause_does_not_decode() {
        let old = json!({"actor": "w/a", "messageId": "m1", "messageType": "Charge", "payload": {},
            "correlationId": null, "from": null, "error": "boom", "attempts": 1});
        assert!(serde_json::from_value::<Failed>(old).is_err());
    }

    #[test]
    fn a_stopped_is_written_with_every_field_and_read_back() {
        for (cause, text) in
            [(StopCause::Stopped, "stopped"), (StopCause::Deleted, "deleted"), (StopCause::Parent, "parent")]
        {
            let full = Stopped { actor: address("worker/w1"), cause, by: Some(address("supervisor/a")) };
            let written = serde_json::to_value(&full).unwrap();
            assert_eq!(written, json!({"actor": "worker/w1", "cause": text, "by": "supervisor/a"}));
            assert_eq!(serde_json::from_value::<Stopped>(written).unwrap(), full);
        }
        let bare = Stopped { actor: address("worker/w1"), cause: StopCause::Deleted, by: None };
        let written = serde_json::to_value(&bare).unwrap();
        assert_eq!(written, json!({"actor": "worker/w1", "cause": "deleted", "by": null}), "by is always written");
        assert_eq!(serde_json::from_value::<Stopped>(written).unwrap(), bare);
    }

    /// Scenario: A prefix at a dot.
    #[test]
    fn a_prefix_matches_at_a_dot() {
        let billing = target("billing");
        assert_eq!(billing, WatchTarget::Types(TypeName::parse("billing").unwrap()));
        assert!(billing.matches_failure(&address("billing/x")));
        assert!(billing.matches_failure(&address("billing.invoices/acme")));
        assert!(!billing.matches_failure(&address("billings/x")));
        assert!(target("billing.invoices").matches_failure(&address("billing.invoices.v2/x")));
        assert!(!target("billing.invoices").matches_failure(&address("billing/x")));
    }

    /// Scenario: Every type.
    #[test]
    fn every_type_matches_every_failure() {
        assert_eq!(target("*"), WatchTarget::Every);
        for actor in ["a/1", "billing.invoices/acme", "z-9/_"] {
            assert!(WatchTarget::Every.matches_failure(&address(actor)), "{actor}");
        }
    }

    /// Scenario: A text that is no target.
    #[test]
    fn a_text_that_is_no_target_is_refused() {
        for bad in ["billing..x/", "a//b", "", "Billing", "**", "*/x", "a/", "/a", "a b"] {
            let e = WatchTarget::parse(bad).unwrap_err();
            assert!(e.to_string().starts_with(&format!("Invalid watch target '{bad}'")), "{e}");
        }
        assert!(serde_json::from_value::<WatchTarget>(json!("a//b")).is_err());
    }

    #[test]
    fn a_target_reads_back_from_its_text() {
        for text in ["billing.invoices/acme", "billing", "*", "files/a%2Fb:c"] {
            let parsed = target(text);
            assert_eq!(parsed.to_string(), text);
            assert_eq!(serde_json::to_value(&parsed).unwrap(), json!(text));
            assert_eq!(serde_json::from_value::<WatchTarget>(json!(text)).unwrap(), parsed);
        }
        assert_eq!(target("worker/w1"), WatchTarget::Actor(address("worker/w1")));
    }

    #[test]
    fn a_removal_matches_the_address_only() {
        let acme = address("billing.invoices/acme");
        assert!(WatchTarget::Actor(acme.clone()).matches_removal(&acme));
        assert!(WatchTarget::Actor(acme.clone()).matches_failure(&acme));
        assert!(!WatchTarget::Actor(address("billing.invoices/other")).matches_removal(&acme));
        assert!(!target("billing.invoices").matches_removal(&acme));
        assert!(!target("billing").matches_removal(&acme));
        assert!(!WatchTarget::Every.matches_removal(&acme));
    }

    #[test]
    fn the_targets_of_a_failure_are_the_address_each_prefix_at_a_dot_and_every_type() {
        assert_eq!(
            failure_targets(&address("billing.invoices.v2/acme")),
            ["billing.invoices.v2/acme", "billing.invoices.v2", "billing.invoices", "billing", "*"]
        );
        assert_eq!(failure_targets(&address("a/1")), ["a/1", "a", "*"]);
        for text in failure_targets(&address("billing.invoices/acme")) {
            assert!(target(&text).matches_failure(&address("billing.invoices/acme")), "{text}");
        }
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    #[allow(dead_code)]
    enum Supervisor {
        /// A variant documented in its own words: descriptions do not count.
        Failed(Failed),
        Stopped(Stopped),
        Restart {
            name: String,
        },
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    #[allow(dead_code, clippy::large_enum_variant)]
    enum OnlyFailed {
        Failed(Failed),
        Restart { name: String },
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    #[allow(dead_code)]
    enum OwnFailed {
        Failed { reason: String },
        Stopped(Stopped),
    }

    /// The `Failed` of the release before 0.14.0: every field but the cause.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct FailedWithoutCause {
        actor: ActorAddress,
        message_id: MessageId,
        message_type: String,
        payload: Value,
        correlation_id: Option<String>,
        from: Option<ActorAddress>,
        error: String,
        attempts: u32,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    #[allow(dead_code)]
    enum OldFailed {
        Failed(FailedWithoutCause),
        Stopped(Stopped),
    }

    /// A type of the family's own that is called like a part of the core's types does not change them.
    mod mine {
        #[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
        #[allow(dead_code)]
        pub enum StopCause {
            Mine,
            Yours,
        }
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    #[allow(dead_code)]
    enum WithOwnTypes {
        Failed(Failed),
        Stopped(Stopped),
        Other { cause: mine::StopCause },
    }

    fn contract_of<M: crate::message::MessageFamily>() -> Contract {
        let messages = crate::schema::message_schemas::<M>().unwrap();
        Contract::new(TypeName::parse("supervisor").unwrap(), json!({}), messages)
    }

    #[test]
    fn a_family_with_both_notifications_may_watch() {
        assert_eq!(watcher_problem(&contract_of::<Supervisor>()), None);
        assert_eq!(watcher_problem(&contract_of::<WithOwnTypes>()), None);
    }

    #[test]
    fn a_family_without_stopped_may_not_watch() {
        let problem = watcher_problem(&contract_of::<OnlyFailed>()).unwrap();
        assert!(problem.contains("has no message type Stopped"), "{problem}");
    }

    #[test]
    fn a_family_with_a_failed_of_its_own_may_not_watch() {
        let problem = watcher_problem(&contract_of::<OwnFailed>()).unwrap();
        assert!(problem.contains("message type Failed that is not cicada::Failed"), "{problem}");
    }

    /// Scenario: A watcher with a `Failed` of its own, for a `Failed` without a cause.
    #[test]
    fn a_family_with_the_failed_of_the_release_before_may_not_watch() {
        let problem = watcher_problem(&contract_of::<OldFailed>()).unwrap();
        assert!(problem.contains("message type Failed that is not cicada::Failed"), "{problem}");
    }
}
