//! Example 11 — debouncing a burst of edits.
//!
//! A [`Document`] is edited many times in a row, and its search entry needs one reindex for the burst, not one per
//! edit. Each edit creates the document's debouncer ([`reindex_debouncer`]) and submits a `Reindex` of the new version
//! to it in one turn; the [`DebouncerActor`](cicada_actors::DebouncerActor) keeps only the latest submission and sends
//! it to the [`Indexer`] once the document was quiet for two seconds, and at the latest ten seconds after the first
//! edit of the burst. The debouncer is the sender of what it forwards, so the reindex carries the address to reply to
//! in its payload, and its retry policy at the indexer is chosen in the submission (see `tests/debounce.rs`). A flush
//! that fails, because the indexer is not there, keeps the latest submission: an operator sends the debouncer a
//! `Flush` once the indexer is back.
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::DebouncerActor;
//! # use cicada_examples::debounce::{DocumentActor, Indexer, IndexerActor, SearchIndex};
//! # async fn run(store: cicada::Store, index: impl SearchIndex) -> Result<(), Box<dyn std::error::Error>> {
//! let system =
//!     ActorSystem::builder(store).host(DebouncerActor).host(DocumentActor).host(IndexerActor { index }).build();
//! system.start().await?;
//! system.create(&Indexer, &cicada::NoState {}).await?;
//! # Ok(()) }
//! ```

use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use cicada_actors::{Debouncer, DebouncerMsg, DebouncerState};
use serde::{Deserialize, Serialize};

/// The debouncer of the reindexes of document `id`, `cicada.debouncer/reindex-<id>`.
pub fn reindex_debouncer(id: &str) -> Debouncer {
    Debouncer::named(format!("reindex-{id}"))
}

/// Two seconds quiet, at most ten seconds from the first edit of a burst.
pub fn reindex_settings() -> DebouncerState {
    DebouncerState::new(Duration::from_secs(2), Duration::from_secs(10)).expect("valid settings")
}

/// One document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// The document id.
    pub id: String,
}

impl ActorRef for Document {
    const TYPE: &'static str = "documents";
    type Message = DocumentMsg;
}

/// Messages of [`Document`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum DocumentMsg {
    /// Replaces the text, counts a version and submits a reindex of it to the debouncer.
    Edit {
        /// The new text.
        text: String,
    },
    /// A version was indexed: the reply of the [`Indexer`].
    Indexed {
        /// The version.
        version: u32,
    },
}

/// State of a [`Document`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DocumentState {
    /// The text.
    pub text: String,
    /// The number of edits.
    pub version: u32,
    /// The last version the indexer reported.
    pub indexed: Option<u32>,
}

/// The document type.
pub struct DocumentActor;

impl Actor for DocumentActor {
    type Ref = Document;
    type State = DocumentState;

    async fn handle(&self, turn: &mut Turn<Self>, message: DocumentMsg) -> HandlerResult {
        match message {
            DocumentMsg::Edit { text } => {
                let state = turn.state_mut();
                state.version += 1;
                state.text = text.clone();
                let reindex = IndexerMsg::Reindex {
                    doc: turn.self_ref().id.clone(),
                    version: turn.state().version,
                    text,
                    reply_to: turn.self_address().clone(),
                };
                let debouncer = reindex_debouncer(&turn.self_ref().id);
                // Creates the debouncer if it is not there yet; one that exists keeps its settings and its burst.
                turn.create(&debouncer, &reindex_settings())?;
                let retry = RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5))?)?;
                turn.send(&debouncer, DebouncerMsg::submit(&Indexer, reindex, Some(retry))?)?;
            }
            DocumentMsg::Indexed { version } => turn.state_mut().indexed = Some(version),
        }
        Ok(())
    }
}

/// The indexer, `indexer/_`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Indexer;

impl ActorRef for Indexer {
    const TYPE: &'static str = "indexer";
    type Message = IndexerMsg;
}

/// Messages of [`Indexer`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum IndexerMsg {
    /// Indexes `text` as `version` of `doc` and tells `reply_to`: the sender of this message is the debouncer, not the
    /// document.
    Reindex {
        /// The document id.
        doc: String,
        /// The version.
        version: u32,
        /// The text.
        text: String,
        /// The document to tell.
        reply_to: ActorAddress,
    },
}

/// The search index cannot be reached.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct IndexError(pub String);

/// The search index.
pub trait SearchIndex: Send + Sync + 'static {
    /// Indexes a document's text; fails when the index cannot be reached.
    fn put(&self, doc: &str, text: &str) -> impl Future<Output = Result<(), IndexError>> + Send;
}

/// The indexer type. A put that fails is tried again by the retry policy the submission chose, without passing the
/// debouncer again.
pub struct IndexerActor<I> {
    /// The search index.
    pub index: I,
}

impl<I: SearchIndex> Actor for IndexerActor<I> {
    type Ref = Indexer;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: IndexerMsg) -> HandlerResult {
        let IndexerMsg::Reindex { doc, version, text, reply_to } = message;
        self.index.put(&doc, &text).await?;
        turn.send_to(&reply_to, &DocumentMsg::Indexed { version }, SendOptions::new()).await?;
        Ok(())
    }
}
