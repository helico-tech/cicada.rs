//! The operations a [`super::FailureWatcher`] needs, offered by a node in-process and by the HTTP client.

use async_trait::async_trait;
use cicada::address::ActorAddress;
use cicada::runtime::{CreateActorResult, SendResult};
use cicada::store::{MessageId, PageCursor};
use cicada::time::Timestamp;
use cicada::{ActorSystem, SendOptions};
use serde_json::Value;

/// An error of an [`ActorOperations`] call.
pub type OperationError = Box<dyn std::error::Error + Send + Sync>;

/// A failed message, as the failures listing (`GET /failures`) serves it.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    /// The failed message.
    pub message_id: MessageId,
    /// Its target.
    pub actor: ActorAddress,
    /// Its type name.
    pub message_type: String,
    /// Its payload.
    pub payload: Value,
    /// Its correlation id.
    pub correlation_id: Option<String>,
    /// Why it failed.
    pub error: Option<String>,
    /// When it failed (store time).
    pub finished_at: Timestamp,
}

/// What a send of an [`ActorOperations`] came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// Enqueued now.
    Accepted,
    /// The id was already accepted.
    Duplicate,
    /// The target does not exist.
    NoSuchInstance,
}

/// The subset of the actor operations the [`super::FailureWatcher`] uses: implemented by [`ActorSystem`] (in-process)
/// and, with the `client` feature, by `cicada_client::ActorClient`, so a watcher can also run in a process of its own
/// and reach the nodes over HTTP, as in the Kotlin implementation.
#[async_trait]
pub trait ActorOperations: Send + Sync + 'static {
    /// Creates `address` with `state`; idempotent.
    async fn create(&self, address: &ActorAddress, state: Value) -> Result<(), OperationError>;

    /// Sends `payload` as `message_type` to `to` with the dedup id `id`.
    async fn send(
        &self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        id: MessageId,
    ) -> Result<Sent, OperationError>;

    /// One page of the failures of instances under `prefix` that finished at or after `since`, oldest first, and the
    /// cursor of the next page.
    async fn failures(
        &self,
        prefix: &str,
        since: Option<Timestamp>,
        cursor: Option<String>,
    ) -> Result<(Vec<Failure>, Option<String>), OperationError>;
}

const PAGE: usize = 100;

#[async_trait]
impl ActorOperations for ActorSystem {
    async fn create(&self, address: &ActorAddress, state: Value) -> Result<(), OperationError> {
        match self.create_actor(address, Some(state)).await? {
            CreateActorResult::Created | CreateActorResult::Exists(_) => Ok(()),
            other => Err(format!("Create of {address} refused: {other:?}").into()),
        }
    }

    async fn send(
        &self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        id: MessageId,
    ) -> Result<Sent, OperationError> {
        match self.send_message(to, message_type, payload, SendOptions::new().id(id)).await? {
            SendResult::Accepted(_) => Ok(Sent::Accepted),
            SendResult::Duplicate(_) => Ok(Sent::Duplicate),
            SendResult::NoSuchInstance => Ok(Sent::NoSuchInstance),
            other => Err(format!("Send to {to} refused: {other:?}").into()),
        }
    }

    async fn failures(
        &self,
        prefix: &str,
        since: Option<Timestamp>,
        cursor: Option<String>,
    ) -> Result<(Vec<Failure>, Option<String>), OperationError> {
        let cursor = cursor.map(PageCursor);
        let page = ActorSystem::failures(self, prefix, since, cursor.as_ref(), PAGE).await?;
        let items = page
            .items
            .into_iter()
            .map(|f| Failure {
                message_id: f.message_id,
                actor: f.actor,
                message_type: f.envelope.message_type,
                payload: f.envelope.payload,
                correlation_id: f.envelope.correlation_id,
                error: f.error,
                finished_at: f.finished_at,
            })
            .collect();
        Ok((items, page.next.map(|c| c.0)))
    }
}

#[cfg(feature = "client")]
#[async_trait]
impl ActorOperations for cicada_client::ActorClient {
    async fn create(&self, address: &ActorAddress, state: Value) -> Result<(), OperationError> {
        self.create_actor(address, Some(state)).await?;
        Ok(())
    }

    async fn send(
        &self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        id: MessageId,
    ) -> Result<Sent, OperationError> {
        match self.send_message(to, message_type, payload, SendOptions::new().id(id)).await {
            Ok(receipt) if receipt.duplicate => Ok(Sent::Duplicate),
            Ok(_) => Ok(Sent::Accepted),
            Err(e) if e.problem().is_some_and(|p| p.problem_type == cicada::api::problem_types::NO_SUCH_INSTANCE) => {
                Ok(Sent::NoSuchInstance)
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn failures(
        &self,
        prefix: &str,
        since: Option<Timestamp>,
        cursor: Option<String>,
    ) -> Result<(Vec<Failure>, Option<String>), OperationError> {
        let page = cicada_client::ActorClient::failures(self, prefix, since, cursor.as_deref(), Some(PAGE)).await?;
        let items = page
            .items
            .into_iter()
            .map(|f| Failure {
                message_id: f.id,
                actor: f.actor,
                message_type: f.message.message_type,
                payload: f.message.payload,
                correlation_id: f.message.correlation_id,
                error: f.error,
                finished_at: f.finished_at,
            })
            .collect();
        Ok((items, page.next))
    }
}
