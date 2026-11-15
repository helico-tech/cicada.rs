//! Example 19 — noticing a nightly import that did not run.
//!
//! A partner drops a file every night, and an [`Import`] actor takes each one in. Nobody notices when the files stop:
//! nothing fails, nothing happens. So each import has a heartbeat watch ([`import_watch`]) that allows 25 hours of
//! silence: an hour of slack on a daily file. `Start` creates the watch and sends it a first `Beat` in one turn, so an
//! import that never runs at all is noticed too; each imported file sends another `Beat`. When 25 hours pass without
//! one, the [`HeartbeatWatchActor`](cicada_actors::HeartbeatWatchActor) sends [`OnCall`] one `ImportSilent` and waits:
//! one silence, one alert. The next file starts a new round without telling anyone; the watch's `status` and
//! `lastBeatAt` show the recovery (see `tests/heartbeat.rs`).
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::HeartbeatWatchActor;
//! # use cicada_examples::heartbeat::{Import, ImportActor, ImportMsg, ImportState, OnCall, OnCallActor, OnCallState};
//! # async fn run(store: cicada::Store) -> Result<(), Box<dyn std::error::Error>> {
//! let system = ActorSystem::builder(store).host(HeartbeatWatchActor).host(ImportActor).host(OnCallActor).build();
//! system.start().await?;
//! system.create(&OnCall, &OnCallState::default()).await?;
//! let import = Import { partner: "acme".into() };
//! system.create(&import, &ImportState::default()).await?;
//! system.send(&import, ImportMsg::Start).await?;
//! # Ok(()) }
//! ```

use std::time::Duration;

use cicada::prelude::*;
use cicada_actors::{HeartbeatWatch, HeartbeatWatchMsg, HeartbeatWatchState};
use serde::{Deserialize, Serialize};

/// How long an import may go without a file: a day and an hour of slack.
pub const ALLOWED_SILENCE: Duration = Duration::from_secs(25 * 3600);

/// The heartbeat watch of the import of `partner`, `cicada.heartbeat-watch/import-<partner>`.
pub fn import_watch(partner: &str) -> HeartbeatWatch {
    HeartbeatWatch::named(format!("import-{partner}"))
}

/// Tells [`OnCall`] that the import of `partner` was silent for [`ALLOWED_SILENCE`].
pub fn import_watch_settings(partner: &str) -> HeartbeatWatchState {
    let alarm = OnCallMsg::ImportSilent { partner: partner.to_owned() };
    HeartbeatWatchState::new(ALLOWED_SILENCE, &OnCall, alarm).expect("valid settings")
}

/// The import of one partner's files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    /// The partner.
    pub partner: String,
}

impl ActorRef for Import {
    const TYPE: &'static str = "imports";
    type Message = ImportMsg;
}

/// Messages of [`Import`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ImportMsg {
    /// Creates the heartbeat watch and sends it a first `Beat`, in one turn: the count starts now, so an import that
    /// never gets a file is noticed too. Safe to repeat: the watch keeps its settings, and the `Beat` restarts its
    /// count.
    Start,
    /// A file was taken in: counts it and tells the watch.
    FileImported {
        /// The file name.
        file: String,
    },
}

/// State of an [`Import`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImportState {
    /// The files taken in.
    pub files: Vec<String>,
}

/// The import type.
pub struct ImportActor;

impl Actor for ImportActor {
    type Ref = Import;
    type State = ImportState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ImportMsg) -> HandlerResult {
        let partner = turn.self_ref().partner.clone();
        let watch = import_watch(&partner);
        match message {
            ImportMsg::Start => turn.create(&watch, &import_watch_settings(&partner))?,
            ImportMsg::FileImported { file } => turn.state_mut().files.push(file),
        }
        turn.send(&watch, HeartbeatWatchMsg::Beat)?;
        Ok(())
    }
}

/// The on-call desk, `ops.on-call/_`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnCall;

impl ActorRef for OnCall {
    const TYPE: &'static str = "ops.on-call";
    type Message = OnCallMsg;
}

/// Messages of [`OnCall`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum OnCallMsg {
    /// The import of `partner` got no file within the allowed silence: the alarm of its heartbeat watch.
    ImportSilent {
        /// The partner.
        partner: String,
    },
}

/// State of [`OnCall`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OnCallState {
    /// One line for each alert, with the store time it was handled.
    pub alerts: Vec<String>,
}

/// The on-call type: keeps the alerts; a real one would page someone.
pub struct OnCallActor;

impl Actor for OnCallActor {
    type Ref = OnCall;
    type State = OnCallState;

    async fn handle(&self, turn: &mut Turn<Self>, message: OnCallMsg) -> HandlerResult {
        let OnCallMsg::ImportSilent { partner } = message;
        let line = format!("{} import silent at {}", partner, cicada::time::format_instant(turn.now()));
        turn.state_mut().alerts.push(line);
        Ok(())
    }
}
