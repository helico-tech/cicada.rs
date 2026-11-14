//! Example 16 — the upgrade of dead letters to 0.17.0.
//!
//! The contract of `cicada.dead-letters` changed in 0.17.0: a letter holds its payload in `payload`, as before, or, when
//! it is deeper than 61 levels, as JSON text in `payloadJson` (ADR 0043). A node of 0.17.0 does not start against the
//! registration of 0.16, so the letters are saved and restored around a new registration. This module is the tested
//! part of that maintenance, run from a management node that hosts nothing (`ActorSystem::builder(store).build()`):
//!
//! 1. [`export`] reads every dead-letters actor: its state, its version, whether its mailbox is paused and the watches
//!    it holds. It refuses an actor with a parent or children, one that another actor watches, and one with a pending
//!    or running message: deleting those would remove other actors or tell watchers it stopped, and work still in
//!    flight would be lost. Write the export to a file with `serde_json`; never pass it through a tool that rounds
//!    numbers.
//! 2. [`convert`] moves each raw payload deeper than 61 levels to `payloadJson` and checks every state against the new
//!    state type and the depth limit, before anything is deleted.
//! 3. [`reset`] checks that the store still holds exactly the exported actors, unchanged, then deletes them, removes the
//!    registration and registers the contract of 0.17.0.
//! 4. [`restore`] creates each actor again at its address with its converted state, pauses the mailboxes that were
//!    paused, sets the watches again and checks the result.
//!
//! What it cannot do is in the operations guide, "Upgrading to 0.17.0": quiescing the senders and the watched work,
//! the backup, stopping every node of 0.16 (a start-time check does not stop a node that runs already), and the
//! restored actors being new instances, with a new version, epoch and creation time.

use std::collections::BTreeSet;

use cicada::actor::contract_of;
use cicada::prelude::*;
use cicada::runtime::limits::MAX_DEPTH;
use cicada::runtime::{OpError, RegisterContractResult, WatchResult};
use cicada::store::RemoveTypeResult;
use cicada::{TypeName, WatchTarget};
use cicada_actors::{DeadLetters, DeadLettersActor, DeadLettersState};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The address prefix of every dead-letters actor.
pub const PREFIX: &str = "cicada.dead-letters/";

/// The deepest payload a letter keeps as a value: it sits three levels into the state.
const DEEPEST_VALUE: usize = MAX_DEPTH - 3;

/// The page size of the reads.
const PAGE: usize = 100;

/// Every dead-letters actor, as [`export`] read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Export {
    /// The actors in address order.
    pub actors: Vec<SavedLetters>,
}

/// One dead-letters actor of an [`Export`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLetters {
    /// Its address.
    pub address: ActorAddress,
    /// Its state, as stored; after [`convert`], as it is restored.
    pub state: Value,
    /// Its version when it was exported: [`reset`] refuses to delete it when the version moved on.
    pub version: i64,
    /// True when its mailbox was paused.
    pub paused: bool,
    /// What it watches, e.g. `*`.
    pub watches: Vec<WatchTarget>,
}

/// Why a step of the upgrade stopped. Nothing is deleted after an error of [`export`], [`convert`] or the checks of
/// [`reset`].
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    /// The actor is part of a tree: deleting it would remove its children too. It needs a migration of its own.
    #[error("{0} has a parent or children: it needs a migration of its own")]
    Tree(ActorAddress),
    /// Another actor watches the actor: deleting it would send that watcher a `Stopped`.
    #[error("{actor} is watched by {watcher}: deleting it would tell the watcher it stopped")]
    Watched {
        /// The dead-letters actor.
        actor: ActorAddress,
        /// Who watches it.
        watcher: ActorAddress,
    },
    /// The actor has a pending or a running message: quiesce the senders and let the mailbox drain first.
    #[error("{0} has pending or running messages: quiesce and drain first")]
    Busy(ActorAddress),
    /// The store and the export differ: an actor is missing from the export, or changed after it.
    #[error("the export is incomplete or stale: {0}")]
    Stale(String),
    /// A state cannot be restored with the contract of 0.17.0.
    #[error("the state of {address} cannot be restored: {why}")]
    Invalid {
        /// The dead-letters actor.
        address: ActorAddress,
        /// Why.
        why: String,
    },
    /// A call of the node failed or was refused.
    #[error("{0}")]
    Node(String),
}

impl From<OpError> for UpgradeError {
    fn from(e: OpError) -> Self {
        Self::Node(e.to_string())
    }
}

/// Reads every dead-letters actor through `system`, which need host nothing, after checking that each is independent,
/// childless, watched by no actor and idle.
pub async fn export(system: &ActorSystem) -> Result<Export, UpgradeError> {
    let mut actors = Vec::new();
    let mut cursor = None;
    loop {
        let page = system.list_actors_with_state(PREFIX, cursor.as_ref(), PAGE).await?;
        for snapshot in page.items {
            let address = snapshot.address;
            let children = system.list_children(&address, None, 1).await?.map_or(0, |page| page.items.len());
            if snapshot.parent.is_some() || children > 0 {
                return Err(UpgradeError::Tree(address));
            }
            let on = WatchTarget::Actor(address.clone());
            if let Some(watch) = system.watches(None, Some(&on), None, 1).await?.items.into_iter().next() {
                return Err(UpgradeError::Watched { actor: address, watcher: watch.watcher });
            }
            let pending = system.pending(&address, None, 1).await?.map_or(0, |page| page.items.len());
            if pending > 0 || system.running_turn(&address).await?.is_some() {
                return Err(UpgradeError::Busy(address));
            }
            let watches = watches_of(system, &address).await?;
            actors.push(SavedLetters {
                address,
                state: snapshot.state,
                version: snapshot.version,
                paused: snapshot.paused,
                watches,
            });
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(Export { actors }),
        }
    }
}

/// Every watch that `watcher` holds.
async fn watches_of(system: &ActorSystem, watcher: &ActorAddress) -> Result<Vec<WatchTarget>, UpgradeError> {
    let mut targets = Vec::new();
    let mut cursor = None;
    loop {
        let page = system.watches(Some(watcher), None, cursor.as_ref(), PAGE).await?;
        targets.extend(page.items.into_iter().map(|w| w.target));
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(targets),
        }
    }
}

/// Moves each raw payload deeper than 61 levels of `export` to `payloadJson`, as compact JSON text, and checks every
/// state: it decodes as the state of 0.17.0 and holds at most 64 levels. Every letter is kept, with its metadata and its
/// `redrivenAs`; a payload too deep to send stays readable as text, and the operator can discard it.
pub fn convert(export: &mut Export) -> Result<(), UpgradeError> {
    for saved in &mut export.actors {
        let invalid = |why: String| UpgradeError::Invalid { address: saved.address.clone(), why };
        let letters = saved.state.get_mut("letters").and_then(Value::as_object_mut);
        for letter in letters.into_iter().flat_map(|letters| letters.values_mut()) {
            let Some(fields) = letter.as_object_mut() else { continue };
            if fields.contains_key("payloadJson") {
                continue;
            }
            if fields.get("payload").is_some_and(|payload| deeper_than(payload, DEEPEST_VALUE)) {
                let text = fields.remove("payload").map(|payload| payload.to_string()).unwrap_or_default();
                fields.insert("payloadJson".to_owned(), Value::String(text));
            }
        }
        if let Err(e) = serde_json::from_value::<DeadLettersState>(saved.state.clone()) {
            return Err(invalid(e.to_string()));
        }
        if deeper_than(&saved.state, MAX_DEPTH) {
            return Err(invalid(format!("the state is nested deeper than {MAX_DEPTH} levels")));
        }
    }
    Ok(())
}

/// Checks that the store holds exactly the actors of `export`, at the versions it read, then deletes them, removes the
/// registration of `cicada.dead-letters` and registers the contract of 0.17.0. Every node of 0.16 must be stopped
/// first.
pub async fn reset(system: &ActorSystem, export: &Export) -> Result<(), UpgradeError> {
    let exported: BTreeSet<&ActorAddress> = export.actors.iter().map(|saved| &saved.address).collect();
    let mut stored = BTreeSet::new();
    let mut cursor = None;
    loop {
        let page = system.list_actors_with_state(PREFIX, cursor.as_ref(), PAGE).await?;
        for snapshot in page.items {
            let Some(saved) = export.actors.iter().find(|saved| saved.address == snapshot.address) else {
                return Err(UpgradeError::Stale(format!("{} is not in the export", snapshot.address)));
            };
            if saved.version != snapshot.version {
                return Err(UpgradeError::Stale(format!("{} changed after the export", snapshot.address)));
            }
            stored.insert(snapshot.address);
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    if let Some(gone) = exported.iter().find(|address| !stored.contains(**address)) {
        return Err(UpgradeError::Stale(format!("{gone} is not in the store")));
    }
    for saved in &export.actors {
        system.delete_actor(&saved.address).await?;
    }
    let name = TypeName::parse(DeadLetters::TYPE).expect("a valid type name");
    match system.remove_type(&name).await? {
        RemoveTypeResult::Removed | RemoveTypeResult::Absent => {}
        RemoveTypeResult::InUse => return Err(UpgradeError::Stale("a dead-letters actor was created again".into())),
    }
    let contract = contract_of::<DeadLettersActor>().map_err(|e| UpgradeError::Node(e.to_string()))?;
    match system.register_contract(&contract).await? {
        RegisterContractResult::Registered(_) => Ok(()),
        other => Err(UpgradeError::Node(format!("the contract of 0.17.0 is not registered: {other:?}"))),
    }
}

/// Creates each actor of the converted `export` again at its address, pauses the mailboxes that were paused and sets
/// the watches again, then reads each back and compares. The actors are new instances: version 0, a new creation time.
pub async fn restore(system: &ActorSystem, export: &Export) -> Result<(), UpgradeError> {
    for saved in &export.actors {
        let address = &saved.address;
        system.create_actor(address, saved.state.clone()).await?;
        if saved.paused {
            system.pause_mailbox(address).await?;
        }
        for target in &saved.watches {
            match system.watch(target, address).await? {
                WatchResult::Created | WatchResult::Exists => {}
                other => return Err(UpgradeError::Node(format!("the watch of {address} on {target}: {other:?}"))),
            }
        }
    }
    for saved in &export.actors {
        let restored = system.get_actor(&saved.address).await?;
        let restored = restored.ok_or_else(|| UpgradeError::Stale(format!("{} was not restored", saved.address)))?;
        let watches = watches_of(system, &saved.address).await?;
        let same_watches = saved.watches.iter().all(|target| watches.contains(target));
        if restored.state != saved.state || restored.paused != saved.paused || !same_watches {
            return Err(UpgradeError::Stale(format!("{} was not restored as exported", saved.address)));
        }
    }
    Ok(())
}

/// True when `value` holds more than `limit` arrays and objects inside each other; empty ones count. Walks with a stack
/// of the open containers, never recursing.
fn deeper_than(value: &Value, limit: usize) -> bool {
    let mut open = vec![(value, 0)];
    while let Some((value, level)) = open.pop() {
        let inner: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            Value::Object(fields) => fields.values().collect(),
            _ => continue,
        };
        if level + 1 > limit {
            return true;
        }
        open.extend(inner.into_iter().map(|inner| (inner, level + 1)));
    }
    false
}
