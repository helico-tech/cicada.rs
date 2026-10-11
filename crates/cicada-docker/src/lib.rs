//! Container handles on Cicada (ADR 0007): one actor per Docker container, at `actor://containers/{name}`.
//!
//! A [`ContainerHandle`] does not hold a running container; it keeps a durable record of one and acts on it through an
//! [`Engine`]. Docker is the truth: every turn inspects the container before it acts, and the container's fixed name
//! (prefix plus handle name) makes every command harmless when a turn runs again.
//!
//! - [`ContainerMsg::Apply`] runs the container with a [`ContainerSpec`], replacing one created from another spec.
//! - [`ContainerMsg::Start`], [`ContainerMsg::Stop`] and [`ContainerMsg::Remove`] act once; `Remove` also deletes the
//!   handle.
//! - The handle checks its container every interval and records an [`Observed`] status in its state; a container that
//!   exits stays exited unless the spec's [`RestartPolicy`] tells Docker otherwise.
//! - Logs never pass through the store: [`container_logs`] streams them from Docker over HTTP.
//!
//! ```no_run
//! # async fn f(system: cicada::ActorSystem) -> Result<(), Box<dyn std::error::Error>> {
//! use cicada_docker::{ContainerHandle, ContainerMsg, ContainerSpec, ContainerState};
//!
//! let web = ContainerHandle::new("web");
//! system.create(&web, &ContainerState::default()).await?;
//! system.send(&web, ContainerMsg::Apply { spec: ContainerSpec::new("nginx:1.27").port(80, 8080) }).await?;
//! # Ok(()) }
//! ```

mod actor;
mod docker;
mod engine;
mod logs;
mod spec;

pub use actor::{ContainerActor, ContainerHandle, ContainerMsg, ContainerState, Observed, SPEC_HASH_LABEL, URI_LABEL};
pub use docker::DockerEngine;
pub use engine::{ContainerInfo, ContainerStatus, Engine, EngineError, LogOptions};
pub use logs::{DOCKER_UNAVAILABLE, container_logs};
pub use spec::{ContainerSpec, PortBinding, Protocol, PullPolicy, RestartPolicy};
