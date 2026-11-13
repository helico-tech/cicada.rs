//! Where a [`Worker`](super::Worker) gets its turns (ADR 0039): the core registration, type, lease, renewal, release and
//! completion operations behind one object-safe trait, with [`ActorSystem`] as the in-process reference source.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::completion::{CompleteTurnResult, TurnReport};
use super::results::{LeaseTurnsResult, OpError, RegisterContractResult, RenewTurnResult};
use super::system::ActorSystem;
use crate::address::{ActorAddress, TypeName};
use crate::contract::Contract;
use crate::store::{MessageId, NodeId, StoreError, TypeDef};

/// A failed call of a [`TurnSource`]. Cloneable, so that a worker can hand the first error that stopped it to every
/// waiter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The source cannot serve the call right now: retrying later may help. A mutation that fails this way may still
    /// have applied.
    #[error("turn source unavailable: {0}")]
    Unavailable(String),
    /// The source refused the request as invalid: retrying the same request cannot help.
    #[error("turn source refused the request: {0}")]
    Invalid(String),
    /// The source cannot serve this worker at all (it could not initialize, or it answered something the worker
    /// cannot use).
    #[error("turn source failed: {0}")]
    Fatal(String),
}

impl From<OpError> for SourceError {
    /// How the reference source classifies an [`OpError`]: invalid input and a store that refuses the data are
    /// `Invalid`, an unavailable or corrupt store is `Unavailable`, a system that cannot start is `Fatal`.
    fn from(e: OpError) -> Self {
        match e {
            OpError::Invalid(why) => SourceError::Invalid(why),
            OpError::Store(StoreError::Invalid(why)) => SourceError::Invalid(why),
            OpError::Store(e @ (StoreError::Unavailable(_) | StoreError::Corrupt(_))) => {
                SourceError::Unavailable(e.to_string())
            }
            OpError::Start(e) => SourceError::Fatal(e.to_string()),
        }
    }
}

/// The operations a [`Worker`](super::Worker) needs from the system that serves its turns: the core results of
/// [`ActorSystem::register_contract`], [`ActorSystem::types`], [`ActorSystem::lease_turns`],
/// [`ActorSystem::renew_turn`], [`ActorSystem::release_turn`] and [`ActorSystem::complete_turn`], with a
/// [`SourceError`]. Implemented by [`ActorSystem`] (in process) and by `Arc<T>`; an HTTP client can implement it too.
///
/// A source is trusted, not an authorization boundary: it must keep the guarantees of the core operations. A lease is
/// fenced by its epoch and handed to one caller only; a renewal never shortens a lease (its expiry is the later of the
/// old one and now plus the TTL); a commit the store turns down is answered `Rejected` with the failure to submit next;
/// and a mutation (lease, renew, release, complete) is attempted once per call: a source does not retry it behind the
/// worker's back, and reports an uncertain result as [`SourceError::Unavailable`]. A worker checks the metadata of each
/// leased batch, but it cannot make a dishonest source keep the store's exclusivity.
#[async_trait]
pub trait TurnSource: Send + Sync {
    /// Registers `contract` without hosting it, as [`ActorSystem::register_contract`].
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError>;

    /// Every registered type with its full definition, as [`ActorSystem::types`] has them.
    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError>;

    /// Leases up to `limit` due turns of `types` (type name to contract hash) for `owner`, as
    /// [`ActorSystem::lease_turns`].
    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError>;

    /// Renews the lease of `actor` under `epoch`, as [`ActorSystem::renew_turn`].
    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError>;

    /// Gives up the lease of `actor` under `epoch` without an outcome, as [`ActorSystem::release_turn`].
    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError>;

    /// Completes the leased turn of `actor` under `epoch` on `message_id` with `report`, as
    /// [`ActorSystem::complete_turn`].
    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError>;
}

#[async_trait]
impl TurnSource for ActorSystem {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        Ok(ActorSystem::register_contract(self, contract).await?)
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        Ok(self.types().await?.into_iter().map(|t| t.definition).collect())
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        Ok(ActorSystem::lease_turns(self, owner, types, limit).await?)
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        Ok(ActorSystem::renew_turn(self, actor, epoch).await?)
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        Ok(ActorSystem::release_turn(self, actor, epoch).await?)
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        Ok(ActorSystem::complete_turn(self, actor, epoch, message_id, report).await?)
    }
}

#[async_trait]
impl<T: TurnSource + ?Sized> TurnSource for Arc<T> {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        (**self).register_contract(contract).await
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        (**self).list_types().await
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        (**self).lease_turns(owner, types, limit).await
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        (**self).renew_turn(actor, epoch).await
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        (**self).release_turn(actor, epoch).await
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        (**self).complete_turn(actor, epoch, message_id, report).await
    }
}
