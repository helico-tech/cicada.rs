use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::time::Timestamp;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};

use crate::engine::{ContainerInfo, ContainerStatus, Engine, LogOptions};
use crate::spec::{ContainerSpec, PullPolicy};

/// Label with the handle's URI; a container without it (or with another) is never touched.
pub const URI_LABEL: &str = "cicada.uri";
/// Label with the hash of the spec the container was created from.
pub const SPEC_HASH_LABEL: &str = "cicada.spec-hash";

/// A handle to one container, `actor://containers/{name}`. The container is named [`ContainerActor`]'s prefix plus
/// `name`; `name` may hold letters, digits, `_`, `.` and `-`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContainerHandle {
    /// The handle's name.
    pub name: String,
}

impl ContainerHandle {
    /// The handle `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ActorRef for ContainerHandle {
    const TEMPLATE: &'static str = "actor://containers/{name}";
    type Message = ContainerMsg;
}

/// Messages of [`ContainerHandle`]. Commands act once and are harmless when repeated; none restarts a container that
/// exited by itself (that is the spec's restart policy). Send commands with a retry policy to ride out a Docker
/// daemon that is briefly unreachable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
#[expect(clippy::large_enum_variant, reason = "a message is decoded once per turn and rarely moved")]
pub enum ContainerMsg {
    /// Run the container with `spec`: create it (pulling the image as the spec's pull policy says) and start it. A
    /// container created from another spec is replaced; one with the same spec is started if it is not running.
    Apply {
        /// What to run.
        spec: ContainerSpec,
    },
    /// Start the container with the last applied spec, creating it when it is missing.
    Start,
    /// Stop the container. Docker kills it after the spec's stop timeout.
    Stop,
    /// Remove the container, then the handle itself.
    Remove,
    /// Look at the container and record what Docker reports. Sent by the handle itself, every check interval.
    Check,
}

/// State of a [`ContainerHandle`]: the last applied spec and the last observation. Docker is the truth; this is a
/// record of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerState {
    /// The last applied spec; `None` until the first `Apply`.
    #[serde(default)]
    pub spec: Option<ContainerSpec>,
    /// What Docker reported at the last look; `None` before the first.
    #[serde(default)]
    pub observed: Option<Observed>,
    /// The pending `Check`.
    #[serde(default)]
    pub next_check: Option<MessageId>,
}

/// One observation of the handle's container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Observed {
    /// Its status; `missing` when there is no container.
    pub status: ContainerStatus,
    /// Docker's container id.
    #[serde(default)]
    pub container_id: Option<String>,
    /// Exit code of the last run, once it has ended (`exited` or `dead`).
    #[serde(default)]
    pub exit_code: Option<i64>,
    /// Docker's error text for the last run.
    #[serde(default)]
    pub error: Option<String>,
    /// Whether the last run was killed for running out of memory.
    #[serde(default)]
    pub oom_killed: bool,
    /// When the last run started.
    #[serde(default)]
    pub started_at: Option<Timestamp>,
    /// When the last run ended.
    #[serde(default)]
    pub finished_at: Option<Timestamp>,
    /// The last lines of output, captured when the container exits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_lines: Vec<String>,
    /// When this observation was made (store time).
    pub checked_at: Timestamp,
    /// Why the last look failed (Docker unreachable, name taken by another container); the other fields are from the
    /// last look that succeeded.
    #[serde(default)]
    pub check_error: Option<String>,
}

/// The container handle type: hosts [`ContainerHandle`]s on one Docker engine.
///
/// ```no_run
/// # use std::sync::Arc;
/// # fn node(store: Arc<dyn cicada::store::Store>) -> Result<(), bollard::errors::Error> {
/// use cicada::ActorSystem;
/// use cicada_docker::{ContainerActor, DockerEngine};
///
/// let containers = ContainerActor::new(DockerEngine::connect()?);
/// let system = ActorSystem::builder(store).host(containers).build();
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct ContainerActor {
    engine: Arc<dyn Engine>,
    prefix: String,
    check_every: Duration,
}

impl std::fmt::Debug for ContainerActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerActor")
            .field("prefix", &self.prefix)
            .field("check_every", &self.check_every)
            .finish_non_exhaustive()
    }
}

impl ContainerActor {
    /// The default container name prefix.
    pub const DEFAULT_PREFIX: &'static str = "cicada-";
    /// The default interval between checks.
    pub const DEFAULT_CHECK_EVERY: Duration = Duration::from_secs(15);
    /// How many lines of output are kept when a container exits.
    pub const LAST_LINES: u32 = 20;
    /// Longest kept output line, in characters.
    const MAX_LINE: usize = 500;

    /// A handle type on `engine`, with the default prefix and check interval.
    pub fn new(engine: impl Engine) -> Self {
        Self::shared(Arc::new(engine))
    }

    /// A handle type on a shared `engine`.
    pub fn shared(engine: Arc<dyn Engine>) -> Self {
        Self { engine, prefix: Self::DEFAULT_PREFIX.to_owned(), check_every: Self::DEFAULT_CHECK_EVERY }
    }

    /// Names containers `prefix` plus the handle name. Nodes that share a Docker daemon but not a store need different
    /// prefixes.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Sets the interval between checks.
    pub fn check_every(mut self, interval: Duration) -> Self {
        self.check_every = interval;
        self
    }

    /// The engine.
    pub fn engine(&self) -> &Arc<dyn Engine> {
        &self.engine
    }

    /// The Docker name of `handle`'s container, or why `handle` has no valid name.
    pub fn container_name(&self, handle: &ContainerHandle) -> Result<String, String> {
        let name = &handle.name;
        let valid = !name.is_empty()
            && name.len() <= 128
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        if !valid {
            return Err(format!(
                "Invalid container handle name '{name}': use 1 to 128 letters, digits, '_', '.' or '-'"
            ));
        }
        let full = format!("{}{name}", self.prefix);
        if !full.starts_with(|c: char| c.is_ascii_alphanumeric()) {
            return Err(format!("Invalid container name '{full}': it must start with a letter or digit"));
        }
        Ok(full)
    }

    /// The container `name` if it belongs to the handle `uri`; an error when the name is taken by another container.
    async fn owned(&self, name: &str, uri: &str) -> Result<Option<ContainerInfo>, HandlerError> {
        match self.engine.inspect(name).await? {
            Some(info) if info.labels.get(URI_LABEL).map(String::as_str) != Some(uri) => Err(HandlerError::permanent(
                format!("Container {name} exists and does not belong to {uri}; remove or rename it"),
            )),
            found => Ok(found),
        }
    }

    async fn create(&self, name: &str, uri: &str, spec: &ContainerSpec) -> HandlerResult {
        let pull = match spec.pull {
            PullPolicy::Always => true,
            PullPolicy::Missing => !self.engine.image_present(&spec.image).await?,
            PullPolicy::Never => false,
        };
        if pull {
            self.engine.pull(&spec.image).await?;
        }
        let labels =
            BTreeMap::from([(URI_LABEL.to_owned(), uri.to_owned()), (SPEC_HASH_LABEL.to_owned(), spec.hash())]);
        self.engine.create(name, spec, &labels).await?;
        Ok(())
    }

    async fn apply(&self, name: &str, uri: &str, spec: &ContainerSpec) -> HandlerResult {
        if let Some(label) = spec.labels.keys().find(|l| l.starts_with("cicada.")) {
            return Err(HandlerError::permanent(format!(
                "Label {label} is reserved: the 'cicada.' prefix is Cicada's"
            )));
        }
        let hash = spec.hash();
        match self.owned(name, uri).await? {
            Some(info) if info.labels.get(SPEC_HASH_LABEL) == Some(&hash) => {}
            Some(_) => {
                self.engine.remove(name).await?;
                self.create(name, uri, spec).await?;
            }
            None => self.create(name, uri, spec).await?,
        }
        self.engine.start(name).await?;
        Ok(())
    }

    /// Records what Docker reports. Never fails: a failed look is recorded in `check_error`, so polling goes on.
    async fn observe(&self, turn: &mut Turn<Self>, name: &str, uri: &str) {
        let now = turn.now();
        let previous = turn.state().observed.clone();
        let observed = match self.owned(name, uri).await {
            Ok(None) => Observed {
                status: ContainerStatus::Missing,
                container_id: None,
                exit_code: None,
                error: None,
                oom_killed: false,
                started_at: None,
                finished_at: None,
                last_lines: Vec::new(),
                checked_at: now,
                check_error: None,
            },
            Ok(Some(info)) => {
                let ended = matches!(info.status, ContainerStatus::Exited | ContainerStatus::Dead);
                let same_run = previous.as_ref().is_some_and(|p| {
                    p.container_id.as_deref() == Some(info.id.as_str()) && p.finished_at == info.finished_at
                });
                let last_lines = match (ended, same_run) {
                    (true, true) => previous.as_ref().map(|p| p.last_lines.clone()).unwrap_or_default(),
                    (true, false) => self.last_lines(name).await,
                    (false, _) => Vec::new(),
                };
                Observed {
                    status: info.status,
                    container_id: Some(info.id),
                    exit_code: info.exit_code.filter(|_| ended), // Docker reports 0 while it runs
                    error: info.error,
                    oom_killed: info.oom_killed,
                    started_at: info.started_at,
                    finished_at: info.finished_at,
                    last_lines,
                    checked_at: now,
                    check_error: None,
                }
            }
            Err(e) => match previous {
                Some(previous) => Observed { checked_at: now, check_error: Some(e.reason()), ..previous },
                None => Observed {
                    status: ContainerStatus::Unknown,
                    container_id: None,
                    exit_code: None,
                    error: None,
                    oom_killed: false,
                    started_at: None,
                    finished_at: None,
                    last_lines: Vec::new(),
                    checked_at: now,
                    check_error: Some(e.reason()),
                },
            },
        };
        turn.state_mut().observed = Some(observed);
    }

    /// The last lines of output; empty when they cannot be read (they are a convenience, not a record).
    async fn last_lines(&self, name: &str) -> Vec<String> {
        let options = LogOptions { tail: Some(Self::LAST_LINES), ..LogOptions::default() };
        let Ok(chunks) = self.engine.logs(name, options).try_collect::<Vec<_>>().await else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(&chunks.concat()).into_owned();
        text.lines().map(|line| line.chars().take(Self::MAX_LINE).collect()).collect()
    }

    /// Cancels the pending `Check` (unless it is this turn's message) and sends the next one.
    fn rearm(&self, turn: &mut Turn<Self>) -> HandlerResult {
        if let Some(pending) = turn.state().next_check.clone()
            && pending != turn.message().id
        {
            turn.cancel(&pending)?;
        }
        let options = SendOptions::new().delay(self.check_every).retention(Retention::REMOVE_ON_DONE);
        let id = turn.send_self_with(ContainerMsg::Check, options)?;
        turn.state_mut().next_check = Some(id);
        Ok(())
    }
}

impl Actor for ContainerActor {
    type Ref = ContainerHandle;
    type State = ContainerState;
    type Message = ContainerMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;
    /// Pulling an image can take minutes.
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(600));

    async fn handle(&self, turn: &mut Turn<Self>, message: ContainerMsg) -> HandlerResult {
        let name = self.container_name(turn.self_ref()).map_err(HandlerError::permanent)?;
        let uri = turn.self_address().to_string();
        match message {
            ContainerMsg::Apply { spec } => {
                self.apply(&name, &uri, &spec).await?;
                turn.state_mut().spec = Some(spec);
            }
            ContainerMsg::Start => {
                let Some(spec) = turn.state().spec.clone() else {
                    return Err(turn.fail(format!("{uri} has no spec yet; send Apply first")));
                };
                if self.owned(&name, &uri).await?.is_none() {
                    self.create(&name, &uri, &spec).await?;
                }
                self.engine.start(&name).await?;
            }
            ContainerMsg::Stop => {
                if self.owned(&name, &uri).await?.is_some() {
                    self.engine.stop(&name).await?;
                }
            }
            ContainerMsg::Remove => {
                if self.owned(&name, &uri).await?.is_some() {
                    self.engine.remove(&name).await?;
                }
                turn.stop();
                return Ok(());
            }
            ContainerMsg::Check => {}
        }
        self.observe(turn, &name, &uri).await;
        self.rearm(turn)
    }
}
