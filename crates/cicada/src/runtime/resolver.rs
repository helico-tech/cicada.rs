//! Resolves addresses through the TypeRegistry and caches parsed contracts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::address::ActorAddress;
use crate::contract::Contract;
use crate::store::{Store, StoreError, StoreResult, TypeDef};
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

/// Finds the type of an address in the TypeRegistry and caches parsed contracts.
pub(crate) struct TypeResolver {
    store: Arc<dyn Store>,
    contracts: Mutex<HashMap<String, Arc<Contract>>>,
}

impl TypeResolver {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store, contracts: Mutex::new(HashMap::new()) }
    }

    pub fn registered(&self, def: TypeDef) -> StoreResult<RegisteredType> {
        let mut cache = self.contracts.lock().unwrap_or_else(|e| e.into_inner());
        let contract = match cache.get(&def.contract_hash) {
            Some(c) => c.clone(),
            None => {
                let parsed =
                    Arc::new(Contract::of(&def).map_err(|e| {
                        StoreError::Corrupt(format!("The contract of {} cannot be read: {e}", def.name))
                    })?);
                cache.insert(def.contract_hash.clone(), parsed.clone());
                parsed
            }
        };
        Ok(RegisteredType { definition: def, contract })
    }

    /// The registered type of `address`, when there is one.
    pub async fn resolve(&self, address: &ActorAddress) -> StoreResult<Option<RegisteredType>> {
        let types = self.store.list_types().await?;
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
