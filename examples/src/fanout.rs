//! Example 7 — fan-out and fan-in.
//!
//! The parent creates one child per item and sends each its work in the same turn (creates are applied before sends,
//! so create-then-send works). The children run in parallel — on any node that hosts them — reply with their result
//! and stop. The parent counts the replies in its state, keyed by child index so a duplicate reply counts once.

use std::collections::BTreeMap;
use std::future::Future;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

/// A batch of items processed in parallel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    /// The batch id.
    pub id: String,
}

impl ActorRef for Batch {
    const TEMPLATE: &'static str = "actor://batches/{id}";
    type Message = BatchMsg;
}

/// Messages of [`Batch`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum BatchMsg {
    /// Fans out one child per item.
    StartBatch {
        /// The inputs, one per child.
        items: Vec<String>,
    },
    /// A child's result.
    ItemDone {
        /// The child's index.
        index: i32,
        /// Its result.
        result: i64,
    },
}

/// Life cycle of a [`Batch`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BatchStatus {
    /// Not started.
    #[default]
    New,
    /// Waiting for children.
    Running,
    /// Every child reported.
    Completed,
}

/// State of a [`Batch`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BatchState {
    /// Where the batch is.
    pub status: BatchStatus,
    /// The number of children.
    pub total: i32,
    /// Result per child index.
    pub results: BTreeMap<i32, i64>,
}

impl BatchState {
    /// Sum of all results so far.
    pub fn sum(&self) -> i64 {
        self.results.values().sum()
    }
}

/// One item of a [`Batch`]; lives only until it reported its result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchItem {
    /// The parent's id.
    pub batch: String,
    /// The item's index in the batch.
    pub index: i32,
}

impl ActorRef for BatchItem {
    const TEMPLATE: &'static str = "actor://batches/{batch}/items/{index}";
    type Message = BatchItemMsg;
}

/// Messages of [`BatchItem`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum BatchItemMsg {
    /// Processes the item and reports to the parent.
    ProcessItem,
}

/// State of a [`BatchItem`]: its input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BatchItemState {
    /// The item's input.
    pub input: String,
}

/// The work done per item.
pub trait ItemProcessor: Send + Sync + 'static {
    /// Processes `input` and returns its result.
    fn process(&self, input: &str) -> impl Future<Output = i64> + Send;
}

/// The parent type.
pub struct BatchActor;

impl Actor for BatchActor {
    type Ref = Batch;
    type State = BatchState;
    type Message = BatchMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: BatchMsg) -> HandlerResult {
        match message {
            BatchMsg::StartBatch { items } => {
                if turn.state().status != BatchStatus::New {
                    return Ok(());
                }
                let batch = turn.self_ref().id.clone();
                for (index, input) in (0..).zip(&items) {
                    let child = BatchItem { batch: batch.clone(), index };
                    turn.create(&child, &BatchItemState { input: input.clone() })?;
                    turn.send(&child, BatchItemMsg::ProcessItem)?;
                }
                let state = turn.state_mut();
                state.status = if items.is_empty() { BatchStatus::Completed } else { BatchStatus::Running };
                state.total = i32::try_from(items.len())?;
            }
            BatchMsg::ItemDone { index, result } => {
                let state = turn.state();
                if state.status != BatchStatus::Running || state.results.contains_key(&index) {
                    return Ok(());
                }
                let state = turn.state_mut();
                state.results.insert(index, result);
                if state.results.len() == state.total as usize {
                    state.status = BatchStatus::Completed;
                }
            }
        }
        Ok(())
    }
}

/// The child type, running `processor`.
pub struct BatchItemActor<P> {
    /// The work per item.
    pub processor: P,
}

impl<P: ItemProcessor> Actor for BatchItemActor<P> {
    type Ref = BatchItem;
    type State = BatchItemState;
    type Message = BatchItemMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: BatchItemMsg) -> HandlerResult {
        match message {
            BatchItemMsg::ProcessItem => {
                let result = self.processor.process(&turn.state().input).await;
                let BatchItem { batch, index } = turn.self_ref().clone();
                turn.send(&Batch { id: batch }, BatchMsg::ItemDone { index, result })?;
                turn.stop(); // the child's job is done; the reply is still sent, in the same commit
            }
        }
        Ok(())
    }
}
