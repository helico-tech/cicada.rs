//! A container runtime for applications on Cicada: the [`ContainerRuntime`] trait, its types and one error type,
//! [`ContainerError`], with a fake, `FakeRuntime`, behind the feature `testing`, and a Docker backend, `DockerRuntime`,
//! behind the feature `docker`. On top of the trait, the container actor, [`ContainerActor`] (ADR 0057): one actor per
//! container under a type name its [`ContainerJob`] chooses, which reconciles the container toward a goal by one armed
//! poll at a time and reports its end once.
//!
//! The runtime's public items name no type of a Docker client, so another backend fits the trait. The crate is an
//! optional layer outside the core (ADR 0056): no library crate of the workspace depends on it.
//!
//! **Every call is safe to repeat.** A container is found by its name, which its owner chooses: [`create`] keeps an
//! existing container, [`start`] never runs an ended one again, and [`stop`] and [`remove`] of a missing one are `Ok`.
//! So a turn that is replayed after a crash calls the runtime again without harm.
//!
//! **Secrets.** [`ContainerSpec::secret_env`] is kept apart from the plain environment: `Debug` shows only its names,
//! and a backend never puts its values on a command line.
//!
//! ```
//! use cicada_containers::{ContainerSpec, Label};
//!
//! let mut spec = ContainerSpec::new("job-1", "alpine:3");
//! spec.command = Some(vec!["echo".into(), "hello".into()]);
//! spec.secret_env.insert("TOKEN".into(), "s3cret".into());
//! spec.labels.insert("run".into(), "r1".into());
//! assert!(!format!("{spec:?}").contains("s3cret"));
//! assert_eq!(Label::new("run", "r1").key, "run");
//! ```
//!
//! [`create`]: ContainerRuntime::create
//! [`start`]: ContainerRuntime::start
//! [`stop`]: ContainerRuntime::stop
//! [`remove`]: ContainerRuntime::remove

mod actor;
// The collector of every runtime; without one of them, nothing calls it yet.
#[cfg_attr(not(any(test, feature = "testing", feature = "docker")), allow(dead_code))]
mod copy;
// The rule of `make_dirs` for every runtime; without one of them, nothing calls it yet.
#[cfg_attr(not(any(test, feature = "testing", feature = "docker")), allow(dead_code))]
mod dirs;
#[cfg(feature = "docker")]
mod docker;
mod error;
#[cfg(any(test, feature = "testing"))]
mod fake;
// The pager of every runtime; without one of them, nothing calls it yet.
#[cfg_attr(not(any(test, feature = "testing", feature = "docker")), allow(dead_code))]
mod logs;
// The rule of `copy_in`, `copy_out` and `make_dirs` for every runtime; without one of them, nothing calls it yet.
#[cfg_attr(not(any(test, feature = "testing", feature = "docker")), allow(dead_code))]
mod paths;
mod runtime;
mod spec;

pub use actor::{
    Container, ContainerActor, ContainerEnd, ContainerEnded, ContainerJob, ContainerMsg, ContainerSettings,
    ContainerState, EndTarget, Goal, Inputs, InvalidContainer, Outcome, Phase,
};
pub use copy::{CopyLimits, CopyOut, SkippedFile};
#[cfg(feature = "docker")]
pub use docker::DockerRuntime;
pub use error::{ContainerError, ErrorKind};
#[cfg(any(test, feature = "testing"))]
pub use fake::{Call, FakeRuntime, Method};
pub use logs::{LOG_PAGE_MAX, LogCursor, LogLine, LogPage, LogStream};
pub use runtime::{ContainerRuntime, ContainerStatus, ContainerSummary, Resource, Skipped, Swept};
pub use spec::{
    ContainerSpec, CopyDir, CopyFile, Label, Mount, MountKind, NetworkAttachment, NetworkSpec, Networking, Tmpfs,
    VolumeSpec,
};
