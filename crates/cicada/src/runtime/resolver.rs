//! Resolves addresses through the TypeRegistry and caches parsed contracts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::source::{SourceError, TurnSource};
use crate::address::ActorAddress;
use crate::contract::Contract;
use crate::store::{Backend, Store, StoreError, StoreResult, TypeDef};
use crate::turn::TurnError;

/// A registered type with its parsed contract.
#[derive(Debug, Clone)]
pub struct RegisteredType {
    /// The registry entry.
    pub definition: TypeDef,
    /// Its parsed contract.
    pub contract: Arc<Contract>,
}

impl RegisteredType {
    /// The type's name.
    pub fn name(&self) -> &crate::address::TypeName {
        &self.definition.name
    }
}

/// Where a resolver reads the TypeRegistry: the store of a node, or the turn source of a worker.
pub(crate) enum Registry {
    Store(Store),
    Source(Arc<dyn TurnSource>),
}

impl Registry {
    /// Every registered type. A source's error becomes the store error a turn reports: `Invalid` stays invalid, and an
    /// unavailable or failed source is an unavailable store (cause `node`).
    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        match self {
            Registry::Store(store) => store.list_types().await,
            Registry::Source(source) => source.list_types().await.map_err(|e| match e {
                SourceError::Invalid(why) => StoreError::Invalid(why),
                e @ (SourceError::Unavailable(_) | SourceError::Fatal(_)) => StoreError::unavailable(e.to_string()),
            }),
        }
    }
}

/// Finds the type of an address in the TypeRegistry and caches parsed contracts, per hash with the JSON they were parsed
/// from: two contracts with one hash may differ in their descriptions, and a type can be removed and registered again
/// with other descriptions by another system. Every lookup reads the registry again; only the parsing is cached.
pub(crate) struct TypeResolver {
    registry: Registry,
    contracts: Mutex<HashMap<String, (Value, Arc<Contract>)>>,
}

impl TypeResolver {
    pub fn new(registry: Registry) -> Self {
        Self { registry, contracts: Mutex::new(HashMap::new()) }
    }

    pub fn registered(&self, def: TypeDef) -> StoreResult<RegisteredType> {
        let mut cache = self.contracts.lock().unwrap_or_else(|e| e.into_inner());
        let contract = match cache.get(&def.contract_hash) {
            Some((json, c)) if *json == def.contract => c.clone(),
            _ => {
                let parsed =
                    Arc::new(Contract::of(&def).map_err(|e| {
                        StoreError::Corrupt(format!("The contract of {} cannot be read: {e}", def.name))
                    })?);
                cache.insert(def.contract_hash.clone(), (def.contract.clone(), parsed.clone()));
                parsed
            }
        };
        Ok(RegisteredType { definition: def, contract })
    }

    /// The registered type of `address`, when there is one.
    pub async fn resolve(&self, address: &ActorAddress) -> StoreResult<Option<RegisteredType>> {
        let types = self.registry.list_types().await?;
        types.into_iter().find(|t| &t.name == address.type_name()).map(|t| self.registered(t)).transpose()
    }

    /// Checks a raw-address send at call time, against the type `to` resolves to.
    pub async fn check_send(&self, to: &ActorAddress, message_type: &str) -> Result<(), TurnError> {
        let rejected =
            |reason: String| TurnError::SendRejected { to: to.clone(), message_type: message_type.to_owned(), reason };
        let resolved =
            self.resolve(to).await?.ok_or_else(|| rejected(format!("No registered actor type {}", to.type_name())))?;
        if !resolved.contract.accepts(message_type) {
            return Err(rejected(format!("{} does not accept message type '{message_type}'", resolved.name())));
        }
        Ok(())
    }
}
