//! The [`OrphanWatch`]: reports the containers of a job that have no container actor, in a task of its own.

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cicada::prelude::{ActorAddress, ActorRef, ActorSystem};
use cicada::runtime::OpError;
use tokio::time::{Instant, sleep_until};

use crate::{Container, ContainerJob, ContainerRuntime, Label, PREFIX_LABEL};

/// The settings of an [`OrphanWatch`], checked by [`OrphanWatch::new`].
///
/// ```
/// use std::time::Duration;
/// use cicada_containers::OrphanSettings;
///
/// let mut settings = OrphanSettings::new();
/// assert_eq!(settings.interval, Duration::from_secs(5 * 60));
/// settings.interval = Duration::from_secs(60);
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanSettings {
    /// The time between rounds; 5 minutes, and more than zero. A container is reported after two rounds without its
    /// actor, so between one and two intervals after its actor is gone.
    pub interval: Duration,
}

impl OrphanSettings {
    /// The defaults.
    pub fn new() -> Self {
        Self { interval: Duration::from_secs(5 * 60) }
    }
}

impl Default for OrphanSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// [`OrphanSettings`] out of their bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidOrphanWatch(String);

/// Reports the containers of a container job that have no container actor: left by a lost store, a test, an actor
/// deleted by hand. It runs as a task, not in actor turns: a host runs [`run`](Self::run) as an app task, once per
/// Docker daemon. It never removes a container; [`sweep`](ContainerRuntime::sweep) and the host decide.
///
/// **Rounds.** Each [`interval`](OrphanSettings::interval), from the start of `run`, a round lists the containers
/// with the label [`PREFIX_LABEL`] and the job's prefix, which the container actor puts on every container it creates.
/// The name without the prefix is the key of its actor, of the type [`ContainerJob::TYPE`], which the round reads with
/// [`ActorSystem::get_actor`].
///
/// **An orphan** is a container whose actor was missing in two rounds in a row, so a read that misses once is no
/// alert. A container actor is saved before it creates its container, so a new container always has its actor.
/// `on_orphan` is called with its name once, while it stays an orphan, and the watch logs a warning too; a host that
/// must send a message spawns a task or uses a channel from it. The watch keeps what it reported in memory, pruned to
/// the listed names: a container whose actor appears is dropped and reported again only after its actor is gone
/// again; one that is removed is forgotten, so a new container of that name starts unreported; after a restart each
/// orphan is reported once more. A labelled container whose name does not start with the prefix, or whose key is no
/// valid actor key, is skipped, with one warning.
///
/// **Errors.** Nothing ends `run` but `stop`. A failed [`list`](ContainerRuntime::list) is logged and tried next round;
/// a failed read of an actor is logged, and that round does not count for the container, either way.
///
/// **Limits.** Two stores on one Docker daemon with the same prefix, such as staging and production, see each other's
/// containers as orphans: give each its own prefix. A container made before 0.22.1 carries no label and is never seen.
///
/// ```
/// use std::sync::{Arc, Mutex};
/// use std::time::Duration;
/// use cicada::prelude::*;
/// use cicada_containers::{
///     ContainerJob, ContainerRuntime, ContainerSpec, FakeRuntime, OrphanSettings, OrphanWatch, PREFIX_LABEL,
/// };
/// use cicada_test::TestActors;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// struct Test {}
/// struct Tests;
///
/// impl ContainerJob for Tests {
///     const TYPE: &'static str = "ci.tests";
///     type Request = Test;
///     fn spec(&self, name: &str, _: &Test) -> Result<ContainerSpec, String> {
///         Ok(ContainerSpec::new(name, "rust:1"))
///     }
/// }
///
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let runtime = FakeRuntime::new();
/// let mut spec = ContainerSpec::new("ci-left", "rust:1");
/// spec.labels.insert(PREFIX_LABEL.into(), "ci-".into()); // as the container actor does
/// runtime.create(&spec).await?; // but no actor ci.tests/left exists
///
/// let orphans = Arc::new(Mutex::new(Vec::new()));
/// let seen = orphans.clone();
/// let watch = OrphanWatch::<Tests>::new(Arc::new(runtime.clone()), "ci-", OrphanSettings::new(), move |name: &str| {
///     seen.lock().unwrap().push(name.to_owned()); // or the host's own alert
/// })?;
/// let test = TestActors::builder().build().await?;
/// watch.run(test.system().clone(), tokio::time::sleep(Duration::from_secs(6 * 60))).await;
/// assert_eq!(*orphans.lock().unwrap(), ["ci-left"]); // at the second round, 5 minutes in
/// assert!(runtime.inspect("ci-left").await.is_ok()); // and not removed
/// # Ok(())
/// # }
/// ```
pub struct OrphanWatch<J> {
    runtime: Arc<dyn ContainerRuntime>,
    label: Label,
    settings: OrphanSettings,
    on_orphan: Box<dyn Fn(&str) + Send + Sync>,
    job: PhantomData<fn() -> J>,
}

impl<J> fmt::Debug for OrphanWatch<J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrphanWatch").field("prefix", &self.label.value).field("settings", &self.settings).finish()
    }
}

impl<J: ContainerJob> OrphanWatch<J> {
    /// A watch of the containers of `runtime` that carry the label [`PREFIX_LABEL`] with the value `prefix`, the
    /// container actor's prefix, which calls `on_orphan` with the name of each orphan. Fails when
    /// [`interval`](OrphanSettings::interval) is zero.
    pub fn new(
        runtime: Arc<dyn ContainerRuntime>,
        prefix: impl Into<String>,
        settings: OrphanSettings,
        on_orphan: impl Fn(&str) + Send + Sync + 'static,
    ) -> Result<Self, InvalidOrphanWatch> {
        if settings.interval.is_zero() {
            return Err(InvalidOrphanWatch("interval is zero".into()));
        }
        Ok(Self {
            runtime,
            label: Label::new(PREFIX_LABEL, prefix.into()),
            settings,
            on_orphan: Box::new(on_orphan),
            job: PhantomData,
        })
    }

    /// Watches in rounds, reading the actors through `system`, until `stop` completes, then returns. No error ends it.
    pub async fn run(self, system: ActorSystem, stop: impl Future<Output = ()>) {
        self.run_on(&system, stop).await;
    }

    async fn run_on(&self, actors: &dyn Actors, stop: impl Future<Output = ()>) {
        let mut stop = pin!(stop);
        let mut rounds = Rounds::default();
        let mut next_round = Instant::now();
        loop {
            tokio::select! {
                () = &mut stop => return,
                () = sleep_until(next_round) => {
                    tokio::select! {
                        () = &mut stop => return,
                        () = self.round(actors, &mut rounds) => next_round = Instant::now() + self.settings.interval,
                    }
                }
            }
        }
    }

    /// Lists the containers and reads the actor of each one, reporting the new orphans.
    async fn round(&self, actors: &dyn Actors, rounds: &mut Rounds) {
        let prefix = self.label.value.as_str();
        let listed = match self.runtime.list(&self.label).await {
            Ok(listed) => listed,
            Err(e) => {
                tracing::warn!(label = %prefix, "container orphans: list failed: {e}");
                return;
            }
        };
        let names: HashSet<&str> = listed.iter().map(|c| c.name.as_str()).collect();
        for set in [&mut rounds.missing, &mut rounds.reported, &mut rounds.skipped] {
            set.retain(|name| names.contains(name.as_str()));
        }
        for container in &listed {
            let name = &container.name;
            if rounds.skipped.contains(name) {
                continue;
            }
            let address = match name.strip_prefix(prefix).map(|key| Container::<J>::named(key).address()) {
                Some(Ok(address)) => address,
                Some(Err(e)) => {
                    tracing::warn!(container = %name, "container orphans: skipped, no actor key: {e}");
                    rounds.skipped.insert(name.clone());
                    continue;
                }
                None => {
                    tracing::warn!(container = %name, label = %prefix, "container orphans: skipped, the name lacks the prefix");
                    rounds.skipped.insert(name.clone());
                    continue;
                }
            };
            match actors.exists(&address).await {
                Ok(true) => {
                    rounds.missing.remove(name);
                    rounds.reported.remove(name);
                }
                Ok(false) if rounds.reported.contains(name) => {}
                Ok(false) => {
                    if rounds.missing.remove(name) {
                        tracing::warn!(container = %name, actor = %address.as_str(), "container orphans: a container without its actor");
                        rounds.reported.insert(name.clone());
                        (self.on_orphan)(name);
                    } else {
                        rounds.missing.insert(name.clone());
                    }
                }
                Err(e) => {
                    tracing::warn!(container = %name, actor = %address.as_str(), "container orphans: the actor read failed: {e}");
                }
            }
        }
    }
}

/// Whether an actor exists: the [`ActorSystem`], or a test's stand-in that fails as scripted.
#[async_trait]
trait Actors: Send + Sync {
    async fn exists(&self, address: &ActorAddress) -> Result<bool, OpError>;
}

#[async_trait]
impl Actors for ActorSystem {
    async fn exists(&self, address: &ActorAddress) -> Result<bool, OpError> {
        Ok(self.get_actor(address).await?.is_some())
    }
}

/// The watch's memory between rounds, each set pruned to the listed names every round.
#[derive(Default)]
struct Rounds {
    /// The containers whose actor was missing in the last round that read it.
    missing: HashSet<String>,
    /// The containers reported as orphans.
    reported: HashSet<String>,
    /// The containers without a key, warned about once.
    skipped: HashSet<String>,
}
