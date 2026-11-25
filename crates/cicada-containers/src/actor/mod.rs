//! The container actor (ADR 0057): a reconciled external resource. A message only records what should be true and
//! arms a poll; one armed poll at a time asks the runtime what is true, takes one step toward the goal, saves it and
//! arms the next. The container's name is the settings' prefix plus the actor's key, so a step that runs again finds
//! the container instead of making a second one. Runtime errors count in the state and back off; they never fail a
//! turn, as a failed poll would be lost.

mod text;
mod wire;

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, MAX_INSTANT, Timestamp};
use cicada::turn::TurnError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use wire::{
    ContainerEnd, ContainerEnded, ContainerMsg, ContainerProgress, ContainerState, EndTarget, FailureKind, Goal,
    InvalidContainer, Outcome, OutputsReport, Phase,
};

use crate::dirs::MAKE;
use crate::paths::path_parts;
use crate::{
    ContainerError, ContainerRuntime, ContainerSpec, ContainerStatus, CopyDir, CopyFile, CopyLimits, ErrorKind,
    LogCursor, OUT_OF_MEMORY, OutputError, OutputErrorKind, OutputStore, SkippedFile,
};

/// The longest container name, prefix and key together; the output store's name rule too.
pub(crate) const MAX_NAME_LENGTH: usize = 200;

/// The environment variable that gives the program its progress file, when its `Start` names a `progressType`.
const PROGRESS_ENV: &str = "CICADA_PROGRESS_FILE";

/// The most bytes of a progress file read; a longer one is skipped.
const MAX_PROGRESS_BYTES: u64 = 64 * 1024;

/// The most log pages one collecting turn reads into the store.
const LOG_PAGES_PER_TURN: usize = 20;

/// The lines of each log page a collecting turn reads: 20 pages of 500, so at most 10 000 lines per turn.
const LOG_PAGE_LINES: u32 = 500;

/// The skipped entries an [`OutputsReport`] lists.
const FIRST_SKIPPED: usize = 10;

/// The label the container actor puts on every container it creates, with its settings' prefix as the value, after
/// the job's [`spec`](ContainerJob::spec), which cannot drop or change it. The [`LogArchiver`](crate::LogArchiver)
/// finds a job's containers by it. A container made before 0.22.1 does not carry it.
pub const PREFIX_LABEL: &str = "cicada.container.prefix";

/// A container job: a host-named actor type whose actors each run one container, built from a small typed request.
///
/// ```
/// use cicada::prelude::*;
/// use cicada_containers::{Container, ContainerJob, ContainerSpec};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// struct Test {
///     /// The commit to test.
///     commit: String,
/// }
///
/// /// No Clone and no Debug needed: a job holds what builds its specs, such as a secret store.
/// struct Tests;
///
/// impl ContainerJob for Tests {
///     const TYPE: &'static str = "ci.tests";
///     type Request = Test;
///
///     fn spec(&self, name: &str, test: &Test) -> Result<ContainerSpec, String> {
///         let mut spec = ContainerSpec::new(name, "rust:1");
///         spec.command = Some(vec!["cargo".into(), "test".into()]);
///         spec.env.insert("COMMIT".into(), test.commit.clone());
///         Ok(spec)
///     }
/// }
///
/// let container = Container::<Tests>::named("t1");
/// assert_eq!(container.address()?.as_str(), "ci.tests/t1");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait ContainerJob: Send + Sync + 'static {
    /// The type name of the actors, chosen by the host.
    const TYPE: &'static str;
    /// The request of one container, fixed at its create and kept in its state; its schema is part of the contract.
    type Request: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static;
    /// The maximum duration of one turn of these actors, their [`Actor::MAX_TURN`]: it wins over the node's default
    /// and over `--max-turn`; `None`, the default, keeps the node's default. Zero is refused when the type is
    /// registered.
    ///
    /// Two turns are long. The create step pulls a missing image, creates the container, makes its folders and copies
    /// its inputs in, all in one turn; a stop waits up to the settings'
    /// [`stop_grace`](ContainerSettings::stop_grace) in one turn. Set the limit above both, or stops time out too.
    ///
    /// With a store ([`ContainerActor::with_store`]), one turn at the end copies the job's
    /// [`outputs`](Self::outputs) folder out and into the store, and each turn before it stores up to 10 000 log
    /// lines: set the limit above the time a copy of the folder's byte limit takes too.
    ///
    /// A turn past the limit ends `failed` (`timeout`) without counting as a runtime error: the poll's own retry
    /// policy runs the step again (7 attempts, waiting 1 to 32 s; every step is safe to repeat, and a container the
    /// cut-off step made is found, not made again). After the last attempt the container waits for the next `Check`,
    /// `Start`, `Stop` or `Remove`. The limit is not part of the contract.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use cicada_containers::{ContainerActor, ContainerJob, ContainerSpec};
    /// # use serde::{Deserialize, Serialize};
    /// use std::time::Duration;
    ///
    /// # #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    /// # struct Test {}
    /// struct Tests;
    ///
    /// impl ContainerJob for Tests {
    ///     const TYPE: &'static str = "ci.tests";
    ///     const MAX_TURN: Option<Duration> = Some(Duration::from_secs(35 * 60)); // a slow pull fits one turn
    ///     type Request = Test;
    ///
    ///     fn spec(&self, name: &str, _: &Test) -> Result<ContainerSpec, String> {
    ///         Ok(ContainerSpec::new(name, "rust:1"))
    ///     }
    /// }
    ///
    /// assert_eq!(<ContainerActor<Tests> as Actor>::MAX_TURN, Some(Duration::from_secs(35 * 60)));
    /// ```
    const MAX_TURN: Option<Duration> = None;

    /// The absolute path of the progress file in the container; `/tmp/cicada-progress` by default. When the container's
    /// `Start` names a `progressType`, the actor sets `CICADA_PROGRESS_FILE` to it in the spec, after the job's
    /// [`spec`](Self::spec), which cannot change it, reads the file at each poll while the container runs and once
    /// when it ends, and sends its text to the end's target as a [`ContainerProgress`] each time it changed. A path
    /// that is relative, has a `..` part or names no file ends the container `failed` with the kind `notStarted`
    /// before anything is made. Progress is best effort: a missing file, one over 64 KiB or not a regular file, and a
    /// failed read send nothing and are not runtime errors.
    ///
    /// The program writes the file whole: to a temporary name in the same folder, then renamed onto the path, as a file
    /// read while it is written can come back half written (the next poll corrects it).
    ///
    /// The file is read with [`copy_out`](ContainerRuntime::copy_out), which cannot see a tmpfs: with a read-only root
    /// file system and a tmpfs on `/tmp`, set this to a path on a volume.
    const PROGRESS_FILE: &'static str = "/tmp/cicada-progress";

    /// The spec of the container `name` for `request`, built at each create, so secrets never reach the store. The actor
    /// sets the spec's name to `name` whatever it returns. An `Err` ends the container `failed` with its text and the
    /// kind `notStarted`, and nothing is made. It is sync: load what it needs before the actor runs.
    ///
    /// A create step first inspects the container: one an earlier step started, whose commit was lost, is adopted
    /// without calling `spec` or [`inputs`](Self::inputs), so a refusal now never ends a running container.
    fn spec(&self, name: &str, request: &Self::Request) -> Result<ContainerSpec, String>;

    /// How long the container of `request` may run before it is stopped and ends `timedOut`, given the settings'
    /// [`timeout`](ContainerSettings::timeout) as `settings`, which it returns by default; `None` is no limit. Asked
    /// once, when the container starts: the deadline, capped at the largest instant, is saved then and never moves,
    /// also when the settings or this method change later. An adopted container gets a fresh deadline, counted from
    /// its adoption.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use cicada_containers::{ContainerJob, ContainerSpec};
    /// # use serde::{Deserialize, Serialize};
    /// use std::time::Duration;
    ///
    /// #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    /// struct Run {
    ///     /// The run's own limit in seconds, or the settings' when none.
    ///     limit: Option<u64>,
    /// }
    ///
    /// struct Runs;
    ///
    /// impl ContainerJob for Runs {
    ///     const TYPE: &'static str = "factory.run";
    ///     type Request = Run;
    ///
    ///     fn spec(&self, name: &str, _: &Run) -> Result<ContainerSpec, String> {
    ///         Ok(ContainerSpec::new(name, "alpine:3"))
    ///     }
    ///
    ///     fn timeout(&self, run: &Run, settings: Option<Duration>) -> Option<Duration> {
    ///         run.limit.map(Duration::from_secs).or(settings)
    ///     }
    /// }
    ///
    /// let ten = Some(Duration::from_secs(10));
    /// assert_eq!(Runs.timeout(&Run { limit: Some(3) }, ten), Some(Duration::from_secs(3)));
    /// assert_eq!(Runs.timeout(&Run { limit: None }, ten), ten);
    /// ```
    fn timeout(&self, request: &Self::Request, settings: Option<Duration>) -> Option<Duration> {
        let _ = request;
        settings
    }

    /// The folders and files the container `name` gets for `request` before it starts; none by default. Like
    /// [`spec`](Self::spec) it is sync and built at each create, after the spec, and never stored, so secrets never
    /// reach the store.
    ///
    /// The actor creates the container, makes the folders ([`make_dirs`](ContainerRuntime::make_dirs)), then writes the
    /// files ([`copy_in`](ContainerRuntime::copy_in)), then starts it, so a file can go into a fresh folder. The step
    /// first inspects the container and adopts one past `Created` without building the inputs, so they go in only
    /// while it is missing or `Created`. One still `Created` gets them again, which is safe: nothing that exists is
    /// made again, and the same files are written. Without inputs the step calls only `inspect`, `create` and `start`.
    ///
    /// The container ends `failed` with the kind `notStarted`, before anything is made, for an `Err`, with its text;
    /// for a path that is relative, has a `..` part or names no file; and for a file at a folder's path or above it
    /// (dirs `/a/b`, file `/a`), as writing it would replace the folder and what is in it. Paths are compared without
    /// empty and `.` parts, so `/out/.` and `/out` are one. It ends so too when the runtime refuses an input
    /// (`InvalidSpec`), finds a file where a folder is asked (`Conflict` while the container is still `Created`), or
    /// has no `make_dirs` (`Unsupported`); the created container then stays until `Remove`. Any other runtime error,
    /// `Other` too, counts and backs off, and the next poll puts the inputs in again.
    ///
    /// Put secret files on the container's own file system, not on a volume: `Remove` deletes the container with its
    /// files, while a named volume outlives it. A tmpfs is refused, as it is mounted only at start.
    fn inputs(&self, name: &str, request: &Self::Request) -> Result<Inputs, String> {
        let _ = (name, request);
        Ok(Inputs::default())
    }

    /// The output folder of the container of `request`, copied into the actor's store at its end; none by default.
    /// Asked in the turn that copies, so only an actor with a store ([`ContainerActor::with_store`]) asks.
    ///
    /// At an end the container ran for, the actor first stores the rest of its log, then copies this folder out
    /// ([`copy_out`](ContainerRuntime::copy_out), with its limits) and puts each file into the store, and the end
    /// carries what was stored and skipped in [`ContainerEnd::outputs`]. Without an outputs folder only the log is
    /// stored, and the counts are 0. A path that is relative, has a `..` part or names no file ends the collection at
    /// once with the error in the report; the end keeps its outcome.
    ///
    /// The copy holds the files in memory, at most [`max_bytes`](CopyLimits::max_bytes) per collecting container:
    /// size the limits against the node's concurrency.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use cicada_containers::{ContainerJob, ContainerSpec, Outputs};
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    /// # struct Test {}
    /// struct Tests;
    ///
    /// impl ContainerJob for Tests {
    ///     const TYPE: &'static str = "ci.tests";
    ///     type Request = Test;
    ///
    ///     fn spec(&self, name: &str, _: &Test) -> Result<ContainerSpec, String> {
    ///         Ok(ContainerSpec::new(name, "rust:1"))
    ///     }
    ///
    ///     fn outputs(&self, _: &Test) -> Option<Outputs> {
    ///         Some(Outputs::new("/work/out")) // at most 64 MiB in 1000 files
    ///     }
    /// }
    ///
    /// let outputs = Tests.outputs(&Test {}).unwrap();
    /// assert_eq!((outputs.limits.max_files, outputs.limits.max_bytes), (1000, 64 * 1024 * 1024));
    /// ```
    fn outputs(&self, request: &Self::Request) -> Option<Outputs> {
        let _ = request;
        None
    }
}

/// The output folder of a container and the limits of its copy, from [`ContainerJob::outputs`]. Make it with
/// [`Outputs::new`] and set the limits by name.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outputs {
    /// The absolute path of the folder, or of one file, in the container.
    pub path: String,
    /// The limits of the copy: 1000 entries and 64 MiB by default.
    pub limits: CopyLimits,
}

impl Outputs {
    /// The folder `path`, with at most 1000 entries and 64 MiB.
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into(), limits: CopyLimits::new(1000, 64 * 1024 * 1024) }
    }
}

/// The folders and files of a container, from [`ContainerJob::inputs`]: put in between its create and its start, the
/// folders first. Make it with [`Inputs::default`] and push to the lists. `Debug` shows no file contents.
///
/// ```
/// use cicada_containers::{CopyDir, CopyFile, Inputs};
///
/// let mut inputs = Inputs::default();
/// let mut out = CopyDir::new("/out");
/// out.mode = 0o777;
/// inputs.dirs.push(out);
/// inputs.files.push(CopyFile::new("/in/config.json", "{}"));
/// assert!(!format!("{inputs:?}").contains("{}"));
/// ```
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    /// The folders to make, each with its mode when it is missing.
    pub dirs: Vec<CopyDir>,
    /// The files to write, after the folders.
    pub files: Vec<CopyFile>,
}

/// Checks the inputs of the container `name` before anything is made: every path parsed by the shared path rule, and
/// no file at a folder's path or above it, as `copy_in` would replace the folder. The reason is the runtime's for a
/// path the rule refuses.
fn check_inputs(name: &str, inputs: &Inputs) -> Result<(), String> {
    let mut dirs = Vec::with_capacity(inputs.dirs.len());
    for dir in &inputs.dirs {
        dirs.push((dir, path_parts(MAKE, name, &dir.path).map_err(|e| e.to_string())?));
    }
    for file in &inputs.files {
        let parts = path_parts("copy into", name, &file.path).map_err(|e| e.to_string())?;
        if let Some((dir, _)) = dirs.iter().find(|(_, dir)| dir.starts_with(&parts)) {
            return Err(format!(
                "the file {} is at or above the folder {}: writing it would replace the folder",
                file.path, dir.path
            ));
        }
    }
    Ok(())
}

/// The container actor of the job `J` keyed `key`, `<J::TYPE>/<key>`: a typed ref whose one string is the key. It needs
/// no `Clone` or `Debug` of `J`.
#[derive(Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct Container<J> {
    /// The key: the end of the container's name. Never reuse it for another job, also after a delete, as a new actor
    /// adopts the old container and its end.
    pub key: String,
    #[serde(skip)]
    job: PhantomData<fn() -> J>,
}

impl<J> Container<J> {
    /// The container actor keyed `key`.
    pub fn named(key: impl Into<String>) -> Self {
        Self { key: key.into(), job: PhantomData }
    }
}

impl<J> Clone for Container<J> {
    fn clone(&self) -> Self {
        Self::named(self.key.clone())
    }
}

impl<J: ContainerJob> fmt::Debug for Container<J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.address() {
            Ok(address) => f.debug_tuple("Container").field(&address.as_str()).finish(),
            Err(_) => f.debug_tuple("Container").field(&self.key).finish(),
        }
    }
}

impl<J: ContainerJob> ActorRef for Container<J> {
    const TYPE: &'static str = J::TYPE;
    type Message = ContainerMsg;
}

/// The settings of a container actor type, the host's: changed without touching actors, though a saved deadline does
/// not move. Make them with [`ContainerSettings::new`] and set the other fields by name.
///
/// ```
/// use std::time::Duration;
/// use cicada_containers::ContainerSettings;
///
/// let mut settings = ContainerSettings::new("ci-")?;
/// settings.timeout = Some(Duration::from_secs(600));
/// assert_eq!(settings.poll_interval, Duration::from_secs(2));
/// assert!(ContainerSettings::new("-ci").is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSettings {
    /// The start of every container's name, before the key: a letter or digit first, then only `A-Za-z0-9_.-`.
    pub prefix: String,
    /// The time between polls of a running container, and the base of the backoff after an error; 2 s. Keep it
    /// positive.
    pub poll_interval: Duration,
    /// The longest wait between polls after errors; 60 s.
    pub max_backoff: Duration,
    /// The runtime errors in a row that end a container without an end `failed`; 10. A removal keeps trying. While the
    /// outputs are collected, they send the pending end with its outcome and the error in its report.
    pub max_errors: u32,
    /// How long a container may run before it is stopped and ends `timedOut`; none. The default of
    /// [`ContainerJob::timeout`], which can change it per request.
    pub timeout: Option<Duration>,
    /// How long a stop waits before it kills the container; 10 s. It runs inside one turn.
    pub stop_grace: Duration,
    /// The most lines of output an end keeps; 200.
    pub log_lines: u32,
    /// The most bytes of output an end keeps, after the lines; 16 KiB.
    pub log_bytes: usize,
    /// How long a container's stored outputs stay in the actor's store after its removal; none: kept until the host
    /// deletes them. With a store ([`ContainerActor::with_store`]), the step that removes the container saves the time
    /// of the deletion, now plus this, capped at the largest instant, and the actor deletes them then with
    /// [`OutputStore::remove`]. The saved time decides: changing or unsetting this later does not move it. Zero
    /// deletes them in the poll after the removal.
    pub output_retention: Option<Duration>,
}

impl ContainerSettings {
    /// The defaults, with names that start with `prefix`. Fails unless it starts with a letter or a digit and holds only
    /// `A-Za-z0-9_.-`.
    pub fn new(prefix: impl Into<String>) -> Result<Self, InvalidContainer> {
        let prefix = prefix.into();
        if !valid_name(&prefix) {
            return Err(InvalidContainer(format!(
                "the prefix {prefix:?} must start with a letter or a digit and hold only A-Za-z0-9_.-"
            )));
        }
        Ok(Self {
            prefix,
            poll_interval: Duration::from_secs(2),
            max_backoff: Duration::from_secs(60),
            max_errors: 10,
            timeout: None,
            stop_grace: Duration::from_secs(10),
            log_lines: 200,
            log_bytes: 16 * 1024,
            output_retention: None,
        })
    }

    /// The name of the container keyed `key`, or why there is none.
    fn name(&self, key: &str) -> Result<String, String> {
        let name = format!("{}{key}", self.prefix);
        if !valid_name(&name) || name.len() > MAX_NAME_LENGTH {
            return Err(format!(
                "the key {key:?} cannot name a container: {name:?} must start with a letter or a digit, hold only \
                 A-Za-z0-9_.- and have at most {MAX_NAME_LENGTH} characters"
            ));
        }
        Ok(name)
    }

    /// The wait before the next poll after `errors` errors in a row: the interval doubled per error, at most
    /// `max_backoff`.
    fn backoff(&self, errors: u32) -> Duration {
        1u32.checked_shl(errors)
            .and_then(|factor| self.poll_interval.checked_mul(factor))
            .map_or(self.max_backoff, |wait| wait.min(self.max_backoff))
    }
}

/// True for a name Docker takes: a letter or digit first, then only `A-Za-z0-9_.-`.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The retry policy of every poll: the ready-made actors' shared one, 7 attempts, exponential from 1 s to 32 s. The end
/// message has it too.
fn retry_policy() -> RetryPolicy {
    let backoff = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(32)).expect("a valid backoff");
    RetryPolicy::new(7, backoff).expect("a valid retry policy")
}

/// The container actor type of the job `J`, under `J::TYPE`, with its runtime and settings.
///
/// Each poll takes one step for its goal and phase: for run, a new container is inspected and adopted when an earlier
/// step started it, else built by the job's spec, created, given the job's inputs and started; a started one is
/// inspected until it ends (started again while created, stopped at its deadline); for stop or remove before the end,
/// it is stopped and inspected, and ends `stopped`, or `timedOut` past its deadline; for remove after the end, it is
/// removed. `ImageNotFound` and `InvalidSpec` from `create`, and `InvalidSpec` from `start`, end it `failed` at once,
/// with the kind `notStarted`. Any other runtime error counts in a row and delays the next poll by the interval
/// doubled per error, up to the maximum; at the maximum errors in a row a container without an end ends `failed`
/// with the kind `runtimeErrors`.
///
/// Limit: a start refused with another error kind, such as `Other`, counts as an error, and the end comes from a later
/// inspect, as `runtimeEnded` when the runtime reports the container `Failed`: a status bears no mark of a refused
/// start.
///
/// With a store ([`with_store`](Self::with_store)), an end the container ran for (`exited`, `failed` as
/// `runtimeEnded` or `outOfMemory`, `stopped`, `timedOut`) is first held as pending in the phase
/// [`Collecting`](Phase::Collecting); the next polls store the rest of its log and its job's outputs, and the poll that
/// finishes records the end with its [`outputs`](ContainerEnd::outputs) and sends it.
///
/// Every node that can claim these actors must reach the same daemon: pin them with a claim owner when only some do
/// (ADR 0052). A create's image pull, a stop's grace and an output copy run inside one turn and its maximum duration,
/// the job's [`MAX_TURN`](ContainerJob::MAX_TURN) or else the node's default.
pub struct ContainerActor<J> {
    job: J,
    runtime: Arc<dyn ContainerRuntime>,
    settings: ContainerSettings,
    store: Option<Arc<dyn OutputStore>>,
}

impl<J: ContainerJob> ContainerActor<J> {
    /// The actor type of `job`, on `runtime`, with `settings`, and without a store.
    pub fn new(job: J, runtime: Arc<dyn ContainerRuntime>, settings: ContainerSettings) -> Self {
        Self { job, runtime, settings, store: None }
    }

    /// This actor type with `store`: at an end the container ran for, the actor stores the rest of its log and its
    /// job's [`outputs`](ContainerJob::outputs) there, keyed by the container's name, before it records and sends the
    /// end (ADR 0066). The store may be the one a [`LogArchiver`](crate::LogArchiver) of the same prefix writes: the
    /// guarded append stores each line once.
    ///
    /// The collection runs in the actor's own turns, so a slow copy holds up only its own container. A runtime or
    /// store error a retry can cure counts as a runtime error and backs off; at the settings' maximum in a row the end
    /// is sent with its outcome and the error in its report. A `Start` during the collection gets the end when it is
    /// done, and a `Remove` removes the container after it.
    ///
    /// A state saved in the phase `Collecting` cannot be read by 0.22: do not roll a node back while one is.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use cicada_containers::{ContainerActor, ContainerJob, ContainerSettings, ContainerSpec, FakeRuntime};
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    /// # struct Test {}
    /// # struct Tests;
    /// # impl ContainerJob for Tests {
    /// #     const TYPE: &'static str = "ci.tests";
    /// #     type Request = Test;
    /// #     fn spec(&self, name: &str, _: &Test) -> Result<ContainerSpec, String> {
    /// #         Ok(ContainerSpec::new(name, "rust:1"))
    /// #     }
    /// # }
    /// use std::sync::Arc;
    /// use cicada_containers::MemoryOutputStore;
    ///
    /// let store = Arc::new(MemoryOutputStore::new());
    /// let actor = ContainerActor::new(Tests, Arc::new(FakeRuntime::new()), ContainerSettings::new("ci-")?)
    ///     .with_store(store);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn with_store(mut self, store: Arc<dyn OutputStore>) -> Self {
        self.store = Some(store);
        self
    }
}

type ContainerTurn<J> = Turn<ContainerActor<J>>;

impl<J: ContainerJob> Actor for ContainerActor<J> {
    type Ref = Container<J>;
    type State = ContainerState<J::Request>;
    const MAX_TURN: Option<Duration> = J::MAX_TURN;

    async fn handle(&self, turn: &mut ContainerTurn<J>, message: ContainerMsg) -> HandlerResult {
        match message {
            ContainerMsg::Start { to, message_type, progress_type } => {
                // Code sends skip the schema: a type the state decoder refuses would make the state unreadable.
                wire::check_type("messageType", &message_type).map_err(HandlerError::permanent)?;
                if let Some(progress_type) = &progress_type {
                    wire::check_type("progressType", progress_type).map_err(HandlerError::permanent)?;
                }
                let target = EndTarget { to, message_type, progress_type };
                let state = turn.state_mut();
                if state.target.is_none() {
                    state.target = Some(target.clone());
                }
                if let Some(end) = state.end.clone() {
                    deliver(turn, &target, end).await?;
                }
                ask(turn, Goal::Run)
            }
            ContainerMsg::Stop => ask(turn, Goal::Stop),
            ContainerMsg::Remove => ask(turn, Goal::Remove),
            ContainerMsg::Check => rearm(turn),
            ContainerMsg::Poll => {
                if turn.state().armed.as_ref() != Some(&turn.message().id) {
                    return Ok(());
                }
                let state = turn.state_mut();
                state.armed = None;
                state.due = None;
                self.step(turn).await
            }
        }
    }
}

/// The progress type of the state's target, when its `Start` named one.
fn progress_type<R>(state: &ContainerState<R>) -> Option<&str> {
    state.target.as_ref()?.progress_type.as_deref()
}

/// True while a poll has something to do: a goal, not removed, and no end yet unless it is to be removed.
fn work_left<R>(state: &ContainerState<R>) -> bool {
    state.goal.is_some() && state.phase != Phase::Removed && (state.end.is_none() || state.goal == Some(Goal::Remove))
}

/// Moves the goal forward to `goal` and arms a poll at once, skipping a backoff; a goal asked again re-arms instead.
fn ask<J: ContainerJob>(turn: &mut ContainerTurn<J>, goal: Goal) -> HandlerResult {
    let state = turn.state_mut();
    if state.goal.is_some_and(|current| current >= goal) {
        return rearm(turn);
    }
    state.goal = Some(goal);
    if !work_left(state) {
        return Ok(());
    }
    let now = turn.current_time();
    arm(turn, now)
}

/// Replaces the armed poll with a fresh one due when it was, or at once when that passed or none is saved; nothing with
/// no work left.
fn rearm<J: ContainerJob>(turn: &mut ContainerTurn<J>) -> HandlerResult {
    if !work_left(turn.state()) {
        return Ok(());
    }
    let now = turn.current_time();
    let due = turn.state().due.filter(|due| *due > now).unwrap_or(now);
    arm(turn, due)
}

/// Cancels the armed poll and arms one due at `at`, with the shared retry policy.
fn arm<J: ContainerJob>(turn: &mut ContainerTurn<J>, at: Timestamp) -> HandlerResult {
    if let Some(old) = turn.state_mut().armed.take()
        && old != turn.message().id
    {
        turn.cancel(&old)?;
    }
    let mut options = SendOptions::new().at(at).retry(retry_policy());
    options.correlation_id = turn.message().correlation_id.clone();
    let id = turn.send_self_with(ContainerMsg::Poll, options).map_err(HandlerError::permanent_from)?;
    let state = turn.state_mut();
    state.armed = Some(id);
    state.due = Some(at);
    Ok(())
}

/// Arms the next poll `wait` from now.
fn arm_after<J: ContainerJob>(turn: &mut ContainerTurn<J>, wait: Duration) -> HandlerResult {
    let now = turn.current_time();
    let at = time::add(now, wait).ok().filter(|at| *at <= MAX_INSTANT).unwrap_or(MAX_INSTANT);
    arm(turn, at)
}

/// Sends `end` to `target`. A send the registry refuses is saved as undelivered, not an error, so a removal is never
/// blocked; a failed read of the registry fails the turn, to be tried again.
async fn deliver<J: ContainerJob>(turn: &mut ContainerTurn<J>, target: &EndTarget, end: ContainerEnd) -> HandlerResult {
    let payload = serde_json::to_value(ContainerEnded { key: turn.self_ref().key.clone(), end })?;
    let options = SendOptions::new().retry(retry_policy());
    match turn.send_raw(&target.to, &target.message_type, payload, options).await {
        Ok(_) => turn.state_mut().undelivered = None,
        Err(e @ TurnError::Store(_)) => return Err(e.into()),
        Err(e) => turn.state_mut().undelivered = Some(text::reason(&e.to_string())),
    }
    Ok(())
}

/// The status's exit code, when it has one.
fn exit_code(status: &ContainerStatus) -> Option<i64> {
    match status {
        ContainerStatus::Exited { code } => Some(*code),
        ContainerStatus::Failed { code, .. } => *code,
        _ => None,
    }
}

impl<J: ContainerJob> ContainerActor<J> {
    /// One step toward the goal, from the phase.
    async fn step(&self, turn: &mut ContainerTurn<J>) -> HandlerResult {
        let state = turn.state();
        let (Some(goal), phase) = (state.goal, state.phase) else {
            return Ok(());
        };
        if phase == Phase::Removed || (phase == Phase::Ended && goal != Goal::Remove) {
            return Ok(());
        }
        let name = match self.settings.name(&turn.self_ref().key) {
            Ok(name) => name,
            // No container can have this name: nothing to remove.
            Err(_) if phase == Phase::Ended => {
                turn.state_mut().phase = Phase::Removed;
                return Ok(());
            }
            // The prefix changed since the end: nothing to collect from.
            Err(reason) if phase == Phase::Collecting => {
                return self.record_pending(turn, OutputsReport::failed(&reason)).await;
            }
            Err(reason) => return self.refuse(turn, reason).await,
        };
        match (goal, phase) {
            (_, Phase::Collecting) => self.collect(turn, &name).await,
            (Goal::Run, Phase::New) => self.create(turn, &name).await,
            (Goal::Run, _) => self.follow(turn, &name).await,
            (_, Phase::Ended) => self.remove(turn, &name).await,
            _ => self.stop(turn, &name).await,
        }
    }

    /// Adopts a container an earlier step started; else builds the spec and the inputs, creates the container, puts
    /// the inputs in, starts it and sets the deadline.
    async fn create(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        match self.runtime.inspect(name).await {
            Ok(ContainerStatus::Missing | ContainerStatus::Created) => {}
            // Started by an earlier run of this step, whose commit was lost: its spec and inputs are not built again.
            Ok(_) => return self.started(turn),
            Err(e) => return self.error(turn, e).await,
        }
        let request = &turn.state().request;
        let progress = progress_type(turn.state()).is_some();
        let built = self.job.spec(name, request).and_then(|spec| {
            let inputs = self.job.inputs(name, request)?;
            check_inputs(name, &inputs)?;
            if progress {
                path_parts("copy out of", name, J::PROGRESS_FILE).map_err(|e| e.to_string())?;
            }
            Ok((spec, inputs))
        });
        let (mut spec, inputs) = match built {
            Ok(built) => built,
            Err(reason) => return self.refuse(turn, reason).await,
        };
        spec.name = name.to_owned();
        // After the job's spec, so a job cannot drop it: the log archiver finds the containers by it.
        spec.labels.insert(PREFIX_LABEL.to_owned(), self.settings.prefix.clone());
        // After the job's spec too, so the program finds the file the actor reads.
        if progress {
            spec.secret_env.remove(PROGRESS_ENV);
            spec.env.insert(PROGRESS_ENV.to_owned(), J::PROGRESS_FILE.to_owned());
        }
        if let Err(e) = self.runtime.create(&spec).await {
            if matches!(e.kind, ErrorKind::ImageNotFound | ErrorKind::InvalidSpec) {
                return self.refuse(turn, e.to_string()).await;
            }
            return self.error(turn, e).await;
        }
        // A container removed since the inspect makes the copies fail with `Conflict`, which `refused` inspects.
        if !inputs.dirs.is_empty()
            && let Err(e) = self.runtime.make_dirs(name, &inputs.dirs).await
        {
            return self.refused(turn, name, e, true).await;
        }
        if !inputs.files.is_empty()
            && let Err(e) = self.runtime.copy_in(name, &inputs.files).await
        {
            return self.refused(turn, name, e, false).await;
        }
        if let Err(e) = self.runtime.start(name).await {
            return self.start_failed(turn, e).await;
        }
        self.started(turn)
    }

    /// Saves the container as started with its deadline from now, and arms the next poll.
    fn started(&self, turn: &mut ContainerTurn<J>) -> HandlerResult {
        let now = turn.current_time();
        let timeout = self.job.timeout(&turn.state().request, self.settings.timeout);
        let deadline = timeout.map(|timeout| {
            time::add(now, timeout).ok().filter(|deadline| *deadline <= MAX_INSTANT).unwrap_or(MAX_INSTANT)
        });
        let state = turn.state_mut();
        state.phase = Phase::Started;
        state.errors = 0;
        state.deadline = deadline;
        arm_after(turn, self.settings.poll_interval)
    }

    /// Handles `error` from `start`: ends the container `failed` as `notStarted` for `InvalidSpec`, else counts it.
    async fn start_failed(&self, turn: &mut ContainerTurn<J>, error: ContainerError) -> HandlerResult {
        if error.kind == ErrorKind::InvalidSpec {
            return self.refuse(turn, error.to_string()).await;
        }
        self.error(turn, error).await
    }

    /// Inspects a started container: follows it while it runs, stops it at its deadline, and records its end.
    async fn follow(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        let status = match self.runtime.inspect(name).await {
            Ok(status) => status,
            Err(e) => return self.error(turn, e).await,
        };
        match status {
            ContainerStatus::Running | ContainerStatus::Created => {
                if status == ContainerStatus::Created
                    && let Err(e) = self.runtime.start(name).await
                {
                    return self.start_failed(turn, e).await;
                }
                turn.state_mut().errors = 0;
                if status == ContainerStatus::Running {
                    self.progress(turn, name).await?;
                }
                let now = turn.current_time();
                if turn.state().deadline.is_some_and(|deadline| now >= deadline) {
                    turn.state_mut().goal = Some(Goal::Stop);
                    return arm(turn, now);
                }
                arm_after(turn, self.settings.poll_interval)
            }
            ContainerStatus::Exited { code } => {
                let Some(logs) = self.logs(turn, name).await? else { return Ok(()) };
                self.last_progress(turn, name).await?;
                let reason = format!("exited with code {code}");
                self.end(turn, Outcome::Exited, None, Some(code), reason, logs, true).await
            }
            ContainerStatus::Failed { code, reason } => {
                let Some(logs) = self.logs(turn, name).await? else { return Ok(()) };
                self.last_progress(turn, name).await?;
                let kind = if reason == OUT_OF_MEMORY { FailureKind::OutOfMemory } else { FailureKind::RuntimeEnded };
                self.fail(turn, kind, code, reason, logs).await
            }
            ContainerStatus::Missing => {
                let reason = "the container went missing".to_owned();
                self.fail(turn, FailureKind::Missing, None, reason, String::new()).await
            }
        }
    }

    /// Stops the container and records its end: `stopped`, or `timedOut` past its deadline; `stopped` without logs when
    /// it is missing, as nothing was made or it is gone.
    async fn stop(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        if let Err(e) = self.runtime.stop(name, self.settings.stop_grace).await {
            return self.error(turn, e).await;
        }
        let status = match self.runtime.inspect(name).await {
            Ok(status) => status,
            Err(e) => return self.error(turn, e).await,
        };
        if status == ContainerStatus::Missing {
            let reason = "stopped; no container was found".to_owned();
            return self.end(turn, Outcome::Stopped, None, None, reason, String::new(), false).await;
        }
        let Some(logs) = self.logs(turn, name).await? else { return Ok(()) };
        self.last_progress(turn, name).await?;
        let now = turn.current_time();
        let (outcome, reason) = match turn.state().deadline {
            Some(deadline) if now >= deadline => {
                (Outcome::TimedOut, format!("ran past its deadline, {}", time::format_instant(deadline)))
            }
            _ => (Outcome::Stopped, "stopped".to_owned()),
        };
        self.end(turn, outcome, None, exit_code(&status), reason, logs, true).await
    }

    /// Removes an ended container.
    async fn remove(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        if let Err(e) = self.runtime.remove(name).await {
            return self.error(turn, e).await;
        }
        let state = turn.state_mut();
        state.phase = Phase::Removed;
        state.errors = 0;
        Ok(())
    }

    /// The container's logs as stored, or `None` when they could not be read and the next poll is armed. A failure
    /// counts like any runtime error; at the maximum in a row the end gets `(no logs: …)` instead. `Unsupported` (a
    /// log the runtime can never read) gets `(no logs: …)` at once, not counted, as a retry cannot help.
    async fn logs(&self, turn: &mut ContainerTurn<J>, name: &str) -> Result<Option<String>, HandlerError> {
        let text = match self.runtime.logs(name, self.settings.log_lines).await {
            Ok(text) => text,
            Err(e) if e.kind == ErrorKind::Unsupported => format!("(no logs: {e})"),
            Err(e) => {
                let state = turn.state_mut();
                state.errors = state.errors.saturating_add(1);
                if state.errors < self.settings.max_errors {
                    let wait = self.settings.backoff(state.errors);
                    arm_after(turn, wait)?;
                    return Ok(None);
                }
                format!("(no logs: {e})")
            }
        };
        Ok(Some(text::logs(&text, self.settings.log_lines, self.settings.log_bytes)))
    }

    /// Reads the progress file once more in the turn that records an end, without a store; with one, the turn that
    /// finishes the collection reads it.
    async fn last_progress(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        if self.store.is_some() {
            return Ok(());
        }
        self.progress(turn, name).await
    }

    /// Reads the progress file of the container `name` and sends its text to the target when it changed since the last
    /// progress sent, with the next `seq`; nothing without a `progressType`. A missing, skipped or unreadable file sends
    /// nothing and is not an error, and an empty text is sent only after one that was not. A send the registry refuses
    /// is not retried and still moves `seq` on; a failed read of the registry fails the turn, to be tried again.
    async fn progress(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        let Some((to, progress_type)) =
            turn.state().target.as_ref().and_then(|t| Some((t.to.clone(), t.progress_type.clone()?)))
        else {
            return Ok(());
        };
        let limits = CopyLimits::new(1, MAX_PROGRESS_BYTES);
        let Ok(out) = self.runtime.copy_out(name, J::PROGRESS_FILE, limits).await else { return Ok(()) };
        // A folder at the path gives the files below it: only the file at the path itself is progress.
        let Ok(parts) = path_parts("copy out of", name, J::PROGRESS_FILE) else { return Ok(()) };
        let path = format!("/{}", parts.join("/"));
        let Some(file) = out.files.into_iter().find(|file| file.path == path) else { return Ok(()) };
        let text = text::plain(&String::from_utf8_lossy(&file.contents));
        let hash: String = Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
        let state = turn.state_mut();
        if state.progress_hash.as_deref() == Some(hash.as_str()) || (state.progress_hash.is_none() && text.is_empty()) {
            return Ok(());
        }
        state.progress_seq = state.progress_seq.saturating_add(1);
        state.progress_hash = Some(hash);
        let seq = state.progress_seq;
        let payload = serde_json::to_value(ContainerProgress { key: turn.self_ref().key.clone(), seq, text })?;
        let options = SendOptions::new().retry(retry_policy());
        match turn.send_raw(&to, &progress_type, payload, options).await {
            Ok(_) => {}
            Err(e @ TurnError::Store(_)) => return Err(e.into()),
            Err(e) => tracing::warn!(container = name, seq, error = %e, "the progress was not sent"),
        }
        Ok(())
    }

    /// Handles `error` from putting the inputs in, of `make_dirs` when `making`, else of `copy_in`: ends the container
    /// `failed` as `notStarted` for `InvalidSpec`, for `Unsupported` from `make_dirs` (a runtime without it, which a
    /// retry cannot help), and for `Conflict` while a new inspect still finds it `Created` (a file where a folder is
    /// asked); else counts it, `Other` too, as a passing daemon error must not end a job for good.
    async fn refused(
        &self,
        turn: &mut ContainerTurn<J>,
        name: &str,
        error: ContainerError,
        making: bool,
    ) -> HandlerResult {
        let refused = match error.kind {
            ErrorKind::InvalidSpec => true,
            ErrorKind::Unsupported => making,
            // The first inspect can be out of date: a container no longer `Created` is a runtime error.
            ErrorKind::Conflict => matches!(self.runtime.inspect(name).await, Ok(ContainerStatus::Created)),
            _ => false,
        };
        if refused {
            return self.refuse(turn, error.to_string()).await;
        }
        self.error(turn, error).await
    }

    /// Counts a runtime error and arms the next poll after the backoff; at the maximum in a row, a container without an
    /// end ends `failed` as `runtimeErrors`, while a removal keeps trying.
    async fn error(&self, turn: &mut ContainerTurn<J>, error: ContainerError) -> HandlerResult {
        let state = turn.state_mut();
        state.errors = state.errors.saturating_add(1);
        let errors = state.errors;
        if errors >= self.settings.max_errors && state.end.is_none() {
            let reason = format!("the runtime failed {errors} times in a row: {error}");
            return self.fail(turn, FailureKind::RuntimeErrors, None, reason, String::new()).await;
        }
        arm_after(turn, self.settings.backoff(errors))
    }

    /// Ends the container `failed` as `notStarted` for `reason`, without code or logs: it never ran.
    async fn refuse(&self, turn: &mut ContainerTurn<J>, reason: String) -> HandlerResult {
        self.fail(turn, FailureKind::NotStarted, None, reason, String::new()).await
    }

    /// Ends the container `failed` with `kind`; it ran for `outOfMemory` and `runtimeEnded`.
    async fn fail(
        &self,
        turn: &mut ContainerTurn<J>,
        kind: FailureKind,
        code: Option<i64>,
        reason: String,
        logs: String,
    ) -> HandlerResult {
        let ran = matches!(kind, FailureKind::OutOfMemory | FailureKind::RuntimeEnded);
        self.end(turn, Outcome::Failed, Some(kind), code, reason, logs, ran).await
    }

    /// Ends the container. `kind` is set exactly for `failed`. With a store and an end the container `ran` for, the end
    /// is held as pending in the phase `Collecting` and a poll is armed at once; else it is recorded and sent now.
    #[allow(clippy::too_many_arguments)]
    async fn end(
        &self,
        turn: &mut ContainerTurn<J>,
        outcome: Outcome,
        kind: Option<FailureKind>,
        code: Option<i64>,
        reason: String,
        logs: String,
        ran: bool,
    ) -> HandlerResult {
        let end = ContainerEnd { outcome, code, kind, reason: text::reason(&reason), logs, outputs: None };
        if ran && self.store.is_some() {
            let state = turn.state_mut();
            state.pending = Some(end);
            state.phase = Phase::Collecting;
            state.errors = 0;
            let now = turn.current_time();
            return arm(turn, now);
        }
        self.record(turn, end).await
    }

    /// Records `end`, sends it to the target in the same commit, and arms the removal when that is the goal.
    async fn record(&self, turn: &mut ContainerTurn<J>, end: ContainerEnd) -> HandlerResult {
        let state = turn.state_mut();
        state.pending = None;
        state.end = Some(end.clone());
        state.phase = Phase::Ended;
        state.errors = 0;
        if let Some(target) = state.target.clone() {
            deliver(turn, &target, end).await?;
        }
        if turn.state().goal == Some(Goal::Remove) {
            let now = turn.current_time();
            return arm(turn, now);
        }
        Ok(())
    }

    /// One collecting step: stores up to 20 pages of the rest of the log, and arms the next poll at once while more
    /// is left; once the log is drained, copies the outputs into the store and finishes. An error a retry can cure
    /// counts and backs off; any other finishes with it in the report.
    async fn collect(&self, turn: &mut ContainerTurn<J>, name: &str) -> HandlerResult {
        let Some(store) = self.store.clone() else {
            // Saved by an actor with a store, now read by one without: the end goes out with nothing collected.
            return self.finish(turn, name, OutputsReport::failed("the actor has no output store")).await;
        };
        let collected = match self.drain(&*store, name).await {
            Ok(true) => self.copy(turn, &*store, name).await,
            Ok(false) => {
                turn.state_mut().errors = 0;
                let now = turn.current_time();
                return arm(turn, now);
            }
            Err(halt) => Err(halt),
        };
        match collected {
            Ok(report) => self.finish(turn, name, report).await,
            Err(Halt::End(error)) => self.finish(turn, name, OutputsReport::failed(&error)).await,
            Err(Halt::Retry(error)) => {
                let state = turn.state_mut();
                state.errors = state.errors.saturating_add(1);
                let errors = state.errors;
                if errors >= self.settings.max_errors {
                    let error = format!("failed {errors} times in a row: {error}");
                    return self.finish(turn, name, OutputsReport::failed(&error)).await;
                }
                arm_after(turn, self.settings.backoff(errors))
            }
        }
    }

    /// Stores up to [`LOG_PAGES_PER_TURN`] pages of the log of `name` after the stored cursor with the guarded append;
    /// true once a short page shows it drained, or when the runtime cannot read the log (`Unsupported`), which skips
    /// it. Another writer of the same store, such as a log archiver, moves the cursor: its `Conflict` reads it again.
    async fn drain(&self, store: &dyn OutputStore, name: &str) -> Result<bool, Halt> {
        let mut cursor: Option<LogCursor> = store.log_end(name).await.map_err(Halt::store)?.cursor;
        for _ in 0..LOG_PAGES_PER_TURN {
            let page = match self.runtime.logs_since(name, cursor, LOG_PAGE_LINES).await {
                Ok(page) => page,
                Err(e) if e.kind == ErrorKind::Unsupported => return Ok(true),
                Err(e) => return Err(Halt::runtime(e)),
            };
            // An empty page leaves the cursor where it was: nothing to write.
            if !page.lines.is_empty() {
                match store.append_logs(name, cursor.as_ref(), &page).await {
                    Ok(_) => {}
                    Err(e) if e.kind == OutputErrorKind::Conflict => {
                        cursor = store.log_end(name).await.map_err(Halt::store)?.cursor;
                        continue;
                    }
                    Err(e) => return Err(Halt::store(e)),
                }
            }
            cursor = Some(page.next);
            if page.lines.len() < LOG_PAGE_LINES as usize {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Copies the job's outputs of `name` out and puts each file into the store, in one turn; a file the store
    /// refuses (`Invalid`) is skipped with its reason. Counts 0 without an outputs folder.
    async fn copy(&self, turn: &ContainerTurn<J>, store: &dyn OutputStore, name: &str) -> Result<OutputsReport, Halt> {
        let Some(outputs) = self.job.outputs(&turn.state().request) else { return Ok(OutputsReport::default()) };
        let out = self.runtime.copy_out(name, &outputs.path, outputs.limits).await.map_err(Halt::runtime)?;
        let mut skipped = out.skipped;
        let (mut stored, mut bytes) = (0u32, 0u64);
        for file in &out.files {
            match store.put_file(name, file).await {
                Ok(()) => {
                    stored = stored.saturating_add(1);
                    bytes = bytes.saturating_add(file.contents.len() as u64);
                }
                Err(e) if e.kind == OutputErrorKind::Invalid => {
                    skipped.push(SkippedFile::new(&file.path, e.to_string()))
                }
                Err(e) => return Err(Halt::Retry(e.to_string())),
            }
        }
        Ok(OutputsReport {
            stored,
            bytes,
            skipped: u32::try_from(skipped.len()).unwrap_or(u32::MAX),
            first_skipped: (skipped.into_iter().take(FIRST_SKIPPED))
                .map(|s| SkippedFile::new(text::reason(&s.path), text::reason(&s.reason)))
                .collect(),
            error: None,
        })
    }

    /// Reads the last progress, then records the pending end with `outputs` and sends it.
    async fn finish(&self, turn: &mut ContainerTurn<J>, name: &str, outputs: OutputsReport) -> HandlerResult {
        self.progress(turn, name).await?;
        self.record_pending(turn, outputs).await
    }

    /// Records the pending end with `outputs` and sends it.
    async fn record_pending(&self, turn: &mut ContainerTurn<J>, outputs: OutputsReport) -> HandlerResult {
        let Some(mut end) = turn.state_mut().pending.take() else { return Ok(()) };
        end.outputs = Some(outputs);
        self.record(turn, end).await
    }
}

/// Why a collecting step stopped early.
enum Halt {
    /// An error a retry can cure: it counts as a runtime error.
    Retry(String),
    /// An error a retry cannot cure: the collection finishes with it.
    End(String),
}

impl Halt {
    /// A runtime error: a bad path (`InvalidSpec`), a missing call (`Unsupported`) and a gone container (`Conflict`)
    /// end the collection; the others are retried.
    fn runtime(error: ContainerError) -> Self {
        match error.kind {
            ErrorKind::InvalidSpec | ErrorKind::Unsupported | ErrorKind::Conflict => Self::End(error.to_string()),
            _ => Self::Retry(error.to_string()),
        }
    }

    /// A store error of the log: `Invalid` ends the collection; the others are retried.
    fn store(error: OutputError) -> Self {
        match error.kind {
            OutputErrorKind::Invalid => Self::End(error.to_string()),
            _ => Self::Retry(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct Nothing {}

    struct Plain;

    impl ContainerJob for Plain {
        const TYPE: &'static str = "test.plain";
        type Request = Nothing;

        fn spec(&self, name: &str, _: &Nothing) -> Result<ContainerSpec, String> {
            Ok(ContainerSpec::new(name, "alpine:3"))
        }
    }

    struct Slow;

    impl ContainerJob for Slow {
        const TYPE: &'static str = "test.slow";
        const MAX_TURN: Option<Duration> = Some(Duration::from_secs(35 * 60));
        type Request = Nothing;

        fn spec(&self, name: &str, _: &Nothing) -> Result<ContainerSpec, String> {
            Ok(ContainerSpec::new(name, "alpine:3"))
        }
    }

    #[test]
    fn the_actors_turn_limit_is_the_jobs() {
        assert_eq!(<ContainerActor<Slow> as Actor>::MAX_TURN, Some(Duration::from_secs(35 * 60)));
        assert_eq!(<ContainerActor<Plain> as Actor>::MAX_TURN, None);
    }

    #[test]
    fn the_prefix_must_be_a_name() {
        for prefix in ["app-", "a", "9.x_y-"] {
            assert!(ContainerSettings::new(prefix).is_ok(), "{prefix}");
        }
        for prefix in ["", "-app", ".app", "_app", "app/", "app ", "äpp"] {
            let refused = ContainerSettings::new(prefix);
            assert!(refused.is_err_and(|e| e.to_string().contains("must start with")), "{prefix}");
        }
    }

    #[test]
    fn the_defaults_are_the_designs() {
        let settings = ContainerSettings::new("app-").unwrap();
        let second = Duration::from_secs(1);
        assert_eq!((settings.poll_interval, settings.max_backoff, settings.max_errors), (2 * second, 60 * second, 10));
        assert_eq!((settings.timeout, settings.stop_grace), (None, 10 * second));
        assert_eq!((settings.log_lines, settings.log_bytes), (200, 16 * 1024));
        assert_eq!(settings.output_retention, None, "kept until the host deletes them");
    }

    #[test]
    fn the_name_is_the_prefix_and_the_key_as_given() {
        let settings = ContainerSettings::new("app-").unwrap();
        assert_eq!(settings.name("j1").unwrap(), "app-j1");
        assert_eq!(settings.name("a.b_c-9").unwrap(), "app-a.b_c-9");
        for key in ["a/b", "a b", "a@b", "é", "a%2Fb"] {
            let refused = settings.name(key);
            assert!(refused.as_ref().is_err_and(|e| e.contains(&format!("{key:?}"))), "{key}: {refused:?}");
        }
        assert!(settings.name(&"k".repeat(MAX_NAME_LENGTH - 4)).is_ok(), "200 characters");
        assert!(settings.name(&"k".repeat(MAX_NAME_LENGTH - 3)).is_err(), "201 characters");
    }

    #[test]
    fn the_backoff_doubles_up_to_its_maximum() {
        let mut settings = ContainerSettings::new("app-").unwrap();
        settings.poll_interval = Duration::from_secs(2);
        settings.max_backoff = Duration::from_secs(60);
        let waits: Vec<u64> = (1..=6).map(|errors| settings.backoff(errors).as_secs()).collect();
        assert_eq!(waits, [4, 8, 16, 32, 60, 60]);
        assert_eq!(settings.backoff(40), settings.max_backoff, "no overflow");
        assert_eq!(settings.backoff(u32::MAX), settings.max_backoff);
    }

    fn inputs(dirs: &[&str], files: &[&str]) -> Inputs {
        Inputs {
            dirs: dirs.iter().map(|path| CopyDir::new(*path)).collect(),
            files: files.iter().map(|path| CopyFile::new(*path, "x")).collect(),
        }
    }

    #[test]
    fn a_file_at_or_above_a_folder_is_refused() {
        for (dirs, file) in [(["/out"], "/out"), (["/a/b"], "/a"), (["/out/."], "/out"), (["/a/b/c"], "/a//./b/")] {
            let refused = check_inputs("app-j1", &inputs(&dirs, &[file])).unwrap_err();
            assert!(refused.contains(&format!("the file {file} is at or above the folder {}", dirs[0])), "{refused}");
        }
    }

    #[test]
    fn folders_and_files_side_by_side_pass() {
        let side = inputs(&["/out", "/in/cache", "/a/b"], &["/in/config.json", "/out/x", "/output", "/a/bc", "/a/b/c"]);
        assert_eq!(check_inputs("app-j1", &side), Ok(()));
        assert_eq!(check_inputs("app-j1", &Inputs::default()), Ok(()));
    }

    #[test]
    fn a_path_the_rule_refuses_is_refused_with_the_runtimes_reason() {
        let refused = check_inputs("app-j1", &inputs(&["out"], &[])).unwrap_err();
        assert_eq!(refused, "invalid spec: make folders in app-j1: relative path out");
        let refused = check_inputs("app-j1", &inputs(&[], &["/in/../etc/passwd"])).unwrap_err();
        assert_eq!(refused, "invalid spec: copy into app-j1: a `..` part in /in/../etc/passwd");
        let refused = check_inputs("app-j1", &inputs(&["/out"], &["/"])).unwrap_err();
        assert_eq!(refused, "invalid spec: copy into app-j1: no file name in /");
    }
}
