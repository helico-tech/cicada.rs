//! Example 12 — batching audit events into bulk writes.
//!
//! Every change to an [`Account`] makes one audit event, and the audit store takes them best in bulk. Each change
//! creates the audit batcher ([`audit_batcher`]) and adds its event to it in one turn; the
//! [`BatcherActor`](cicada_actors::BatcherActor) collects the events in handling order and sends them to the
//! [`AuditLog`] as one `Write { items }` once it holds 50, and at the latest five seconds after the first event of the
//! batch. The retry policy of each write at the audit log is fixed in the batcher's settings (see
//! `tests/batching.rs`). A flush that fails, because the audit log is not there, keeps the events: an operator sends the
//! batcher a `Flush` once the audit log is back.
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::BatcherActor;
//! # use cicada_examples::batching::{AccountActor, AuditLog, AuditLogActor, AuditStore};
//! # async fn run(store: cicada::Store, audit: impl AuditStore) -> Result<(), Box<dyn std::error::Error>> {
//! let system =
//!     ActorSystem::builder(store).host(BatcherActor).host(AccountActor).host(AuditLogActor { audit }).build();
//! system.start().await?;
//! system.create(&AuditLog, &cicada::NoState {}).await?;
//! # Ok(()) }
//! ```

use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use cicada_actors::{Batcher, BatcherMsg, BatcherState};
use serde::{Deserialize, Serialize};

/// The batcher of the audit events, `cicada.batcher/audit`.
pub fn audit_batcher() -> Batcher {
    Batcher::named("audit")
}

/// A `Write` to the audit log of at most 50 events, at most five seconds from the first; each write tried three times.
pub fn audit_settings() -> BatcherState {
    let mut settings = BatcherState::new(&AuditLog, "Write", 50, Duration::from_secs(5)).expect("valid settings");
    let backoff = Backoff::fixed(Duration::from_secs(5)).expect("a valid backoff");
    settings.retry = Some(RetryPolicy::new(3, backoff).expect("a valid retry policy"));
    settings
}

/// One account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// The account id.
    pub id: String,
}

impl ActorRef for Account {
    const TYPE: &'static str = "accounts";
    type Message = AccountMsg;
}

/// Messages of [`Account`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum AccountMsg {
    /// Renames the account and adds an audit event of it to the batcher.
    Rename {
        /// The new name.
        name: String,
    },
}

/// State of an [`Account`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AccountState {
    /// The name.
    pub name: String,
}

/// One audit event: an item of a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditEvent {
    /// The account the event is about.
    pub account: String,
    /// What happened.
    pub action: String,
}

/// The account type.
pub struct AccountActor;

impl Actor for AccountActor {
    type Ref = Account;
    type State = AccountState;

    async fn handle(&self, turn: &mut Turn<Self>, message: AccountMsg) -> HandlerResult {
        let AccountMsg::Rename { name } = message;
        let event = AuditEvent { account: turn.self_ref().id.clone(), action: format!("renamed to {name}") };
        turn.state_mut().name = name;
        // Creates the batcher if it is not there yet; one that exists keeps its settings and its batch.
        turn.create(&audit_batcher(), &audit_settings())?;
        turn.send(&audit_batcher(), BatcherMsg::add(&event)?)?;
        Ok(())
    }
}

/// The audit log, `audit-log/_`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLog;

impl ActorRef for AuditLog {
    const TYPE: &'static str = "audit-log";
    type Message = AuditLogMsg;
}

/// Messages of [`AuditLog`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum AuditLogMsg {
    /// Writes a batch of events in one call: what the batcher sends, `{"items": [...]}`.
    Write {
        /// The events, in the order the batcher handled them.
        items: Vec<AuditEvent>,
    },
}

/// The audit store cannot be reached.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct AuditError(pub String);

/// The audit store.
pub trait AuditStore: Send + Sync + 'static {
    /// Writes events in one call; fails when the store cannot be reached.
    fn write_all(&self, events: &[AuditEvent]) -> impl Future<Output = Result<(), AuditError>> + Send;
}

/// The audit log type. A write that fails is tried again by the retry policy of the batcher's settings, without
/// passing the batcher again.
pub struct AuditLogActor<S> {
    /// The audit store.
    pub audit: S,
}

impl<S: AuditStore> Actor for AuditLogActor<S> {
    type Ref = AuditLog;
    type State = NoState;

    async fn handle(&self, _turn: &mut Turn<Self>, message: AuditLogMsg) -> HandlerResult {
        let AuditLogMsg::Write { items } = message;
        self.audit.write_all(&items).await?;
        Ok(())
    }
}
