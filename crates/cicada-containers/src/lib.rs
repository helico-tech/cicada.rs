//! A container runtime for applications on Cicada: the [`ContainerRuntime`] trait, its types and one error type,
//! [`ContainerError`], with a fake, `FakeRuntime`, behind the feature `testing`, and a Docker backend, `DockerRuntime`,
//! behind the feature `docker`.
//!
//! The crate needs nothing from Cicada and its public items name no type of a Docker client, so another backend fits
//! the trait. It is an optional layer outside the core (ADR 0056): no library crate of the workspace depends on it.
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

#[cfg(feature = "docker")]
mod docker;
mod error;
#[cfg(any(test, feature = "testing"))]
mod fake;
mod runtime;
mod spec;

#[cfg(feature = "docker")]
pub use docker::DockerRuntime;
pub use error::{ContainerError, ErrorKind};
#[cfg(any(test, feature = "testing"))]
pub use fake::{Call, FakeRuntime, Method};
pub use runtime::{ContainerRuntime, ContainerStatus, ContainerSummary, Resource, Skipped, Swept};
pub use spec::{
    ContainerSpec, CopyFile, Label, Mount, MountKind, NetworkAttachment, NetworkSpec, Networking, Tmpfs, VolumeSpec,
};
