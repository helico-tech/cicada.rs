//! The example of the architecture's "Definition API", with the external API as a field of the actor instead of a
//! global. A `Run` starts an import; each page is its own turn (durable steps); the last page reports to the tenant's
//! [`Reports`] actor and schedules the next `Run` an hour later (a cron chain in one line).

use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

/// The importer of one tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sync {
    /// The tenant.
    pub tenant: String,
}

impl ActorRef for Sync {
    const TYPE: &'static str = "sync";
    type Message = SyncMsg;
}

/// Messages of [`Sync`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum SyncMsg {
    /// Starts an import run.
    Run,
    /// Imports the page at `cursor` (`None` = the first page).
    FetchPage {
        /// Where the page starts.
        cursor: Option<String>,
    },
}

/// State of a [`Sync`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncState {
    /// Where to import from.
    pub source: String,
    /// Items imported by the current run.
    #[serde(default)]
    pub imported: i32,
}

/// Collects the results of a tenant's sync runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reports {
    /// The tenant.
    pub tenant: String,
}

impl ActorRef for Reports {
    const TYPE: &'static str = "reports";
    type Message = ReportsMsg;
}

/// Messages of [`Reports`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ReportsMsg {
    /// A sync run finished with `imported` items.
    SyncFinished {
        /// Items imported by the run.
        imported: i32,
    },
}

/// State of [`Reports`]: the item count of every run, oldest first (at most 100).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReportsState {
    /// Item count per run.
    pub runs: Vec<i32>,
}

/// One page of items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Items on the page.
    pub size: i32,
    /// The cursor of the next page, `None` on the last.
    pub next: Option<String>,
}

/// The external API a [`Sync`] imports from.
pub trait SyncApi: Send + std::marker::Sync + 'static {
    /// Fetches the page at `cursor` of `source`; `key` is the idempotency key (calls are at least once).
    fn fetch(&self, source: &str, cursor: Option<&str>, key: &str) -> impl Future<Output = Page> + Send;
}

/// The sync type, importing through `api`.
pub struct SyncActor<A> {
    /// The external API.
    pub api: A,
}

impl<A: SyncApi> Actor for SyncActor<A> {
    type Ref = Sync;
    type State = SyncState;
    type Message = SyncMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail; // mandatory: Fail | Drop | Custom
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(5 * 60));

    async fn handle(&self, turn: &mut Turn<Self>, message: SyncMsg) -> HandlerResult {
        match message {
            SyncMsg::Run => {
                turn.state_mut().imported = 0;
                turn.send_self(SyncMsg::FetchPage { cursor: None })?;
            }
            SyncMsg::FetchPage { cursor } => {
                // External call: at-least-once, so the message id is the idempotency key.
                let page = self.api.fetch(&turn.state().source, cursor.as_deref(), turn.message().id.as_str()).await;
                turn.state_mut().imported += page.size;

                if let Some(next) = page.next {
                    turn.send_self(SyncMsg::FetchPage { cursor: Some(next) })?;
                } else {
                    let reports = Reports { tenant: turn.self_ref().tenant.clone() };
                    let imported = turn.state().imported;
                    // The compiler checks that Reports accepts SyncFinished.
                    turn.send(&reports, ReportsMsg::SyncFinished { imported })?;
                    // The next run; this line is the whole schedule.
                    turn.send_self_with(SyncMsg::Run, SendOptions::new().delay(Duration::from_secs(3600)))?;
                }
            }
        }
        Ok(())
    }
}

/// The reports type.
pub struct ReportsActor;

impl Actor for ReportsActor {
    type Ref = Reports;
    type State = ReportsState;
    type Message = ReportsMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: ReportsMsg) -> HandlerResult {
        match message {
            ReportsMsg::SyncFinished { imported } => {
                let runs = &mut turn.state_mut().runs;
                runs.push(imported);
                if runs.len() > 100 {
                    runs.drain(..runs.len() - 100);
                }
            }
        }
        Ok(())
    }
}
