//! Example 18 — a job that owns a sandbox, and its cleanup when it is cancelled.
//!
//! [`SandboxJob`] reserves capacity, starts a sandbox that runs the job's workload, waits for the workload to finish,
//! then releases the capacity and reports to its owner. Each effect is a step, a request or a send. The reservation's
//! step returns its own idempotency key, `example.sandbox-job/<job>/reserve`: the [`Capacity`] service keys the
//! reservation by it, so a retried step reserves once.
//!
//! A host cancels a run by sending it `FlowMsg::Cancel`. The `Cancel` turn marks the run cancelling and arms the
//! cleanup's `Continue`; no main code runs again. The cleanup, [`Flow::on_cancel`], runs over a journal of its own:
//!
//! 1. It looks the start up in the main journal. No saved request means that its send never committed, so no sandbox
//!    exists. [`Cleanup::main_reply`] tells whether the main code had decided on the workload's reply: only then is
//!    the sandbox known to be gone. Otherwise, also when a `Finished` reply came after the `Cancel`, it sends the
//!    sandbox `Stop` and waits for `Stopped`, which the sandbox sends only once its container is removed. The answer
//!    of `main_reply` never changes during the cleanup, so a reply that comes between two cleanup turns cannot switch
//!    the branch taken.
//! 2. Only then does it release the capacity, by [`Cleanup::main_key`] of the reservation: that key exists also when
//!    the reservation happened but its step failed before its result was saved. A missing main result does not prove
//!    that nothing was reserved.
//! 3. It reports the cancellation to the owner with a send. That send commits in a turn before the one that marks the
//!    run cancelled: the report and the status are not one commit.
//!
//! Both owners fence late work. A [`Sandbox`] that was stopped ignores a later `Start`, such as an operator's retry of
//! a failed one, and creates nothing; [`Capacity::release`] keeps a tombstone, so a late reserve of a released key is
//! refused. The library promises neither: a cleanup by key is safe only with owners that settle such races (see
//! `tests/flow_resources.rs`). Every host and worker of the flow type must run this cleanup before a run is cancelled:
//! a release before 0.18.5, or code with the default cleanup, marks the run cancelled without it.
//!
//! A host registers the three types, starts a run, and cancels it later:
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::{FlowActor, FlowMsg, FlowState};
//! # use cicada_examples::flow_resources::{Capacity, Containers, JobInput, NoticesActor, SandboxActor, SandboxJob, job};
//! # async fn run<C: Capacity, R: Containers>(store: cicada::Store, capacity: C, containers: R)
//! # -> Result<(), Box<dyn std::error::Error>> {
//! let system = ActorSystem::builder(store)
//!     .host(FlowActor::new(SandboxJob { capacity }))
//!     .host(SandboxActor { containers })
//!     .host(NoticesActor)
//!     .build();
//! system.start().await?;
//! let run = job::<C>("j1");
//! system.create(&run, &FlowState::new(JobInput { job: "j1".into(), owner: "team-a".into() })).await?;
//! system.send(&run, FlowMsg::Continue).await?;
//! // Later: stop the job. Read the run's status to learn when its cleanup is done.
//! system.send(&run, FlowMsg::Cancel).await?;
//! # Ok(()) }
//! ```

use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use cicada_actors::{Cleanup, Flow, FlowRun, MainReply, ReplyTo, Step, Target};
use serde::{Deserialize, Serialize};

use crate::steps::ServiceError;

/// The capacity service: one slot per reservation key.
pub trait Capacity: Send + Sync + 'static {
    /// Reserves a slot under `key`; a slot reserved already stays. A key released before is refused.
    fn reserve(&self, key: &str) -> impl Future<Output = Result<(), ServiceError>> + Send;
    /// Releases the slot of `key`, if any, and keeps a tombstone of `key`, so that a late reserve is refused.
    fn release(&self, key: &str) -> impl Future<Output = Result<(), ServiceError>> + Send;
}

/// The container runtime of the sandboxes; both calls are idempotent by key.
pub trait Containers: Send + Sync + 'static {
    /// Creates the container `key`.
    fn create(&self, key: &str) -> impl Future<Output = Result<(), ServiceError>> + Send;
    /// Removes the container `key`, if it exists.
    fn remove(&self, key: &str) -> impl Future<Output = Result<(), ServiceError>> + Send;
}

/// A sandbox, `example.sandboxes/<job>`: the actor that owns one job's container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sandbox(pub String);

impl ActorRef for Sandbox {
    const TYPE: &'static str = "example.sandboxes";
    type Message = SandboxMsg;
}

/// What a sandbox's workload produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Finished {
    /// The workload's output.
    pub output: String,
}

/// A sandbox's confirmation that its container is removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Stopped {
    /// The container that was removed.
    pub container: String,
}

/// The messages of a [`Sandbox`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum SandboxMsg {
    /// Creates the container and starts the workload, which replies [`Finished`] through `reply`; a stopped sandbox
    /// ignores it.
    Start {
        /// Where the workload's result goes.
        reply: ReplyTo<Finished>,
    },
    /// The workload ended with `output`: removes the container and replies to the start.
    Finish {
        /// The workload's output.
        output: String,
    },
    /// Removes the container, fences later starts and replies [`Stopped`] through `reply`; idempotent.
    Stop {
        /// Where the confirmation goes.
        reply: ReplyTo<Stopped>,
    },
}

/// The state of a [`Sandbox`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SandboxState {
    /// The route of the running workload's reply.
    #[serde(default)]
    pub started: Option<ReplyTo<Finished>>,
    /// Whether the sandbox was stopped: a tombstone that fences a late start.
    #[serde(default)]
    pub stopped: bool,
}

/// The sandbox actor, calling `containers`.
pub struct SandboxActor<R> {
    /// The container runtime.
    pub containers: R,
}

impl<R: Containers> Actor for SandboxActor<R> {
    type Ref = Sandbox;
    type State = SandboxState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SandboxMsg) -> HandlerResult {
        let container = turn.self_address().to_string();
        match message {
            SandboxMsg::Start { reply } => {
                if turn.state().stopped {
                    return Ok(()); // a late start, fenced: nothing is created
                }
                self.containers.create(&container).await?;
                turn.state_mut().started = Some(reply);
            }
            SandboxMsg::Finish { output } => {
                let Some(reply) = turn.state().started.clone() else {
                    return Ok(());
                };
                self.containers.remove(&container).await?;
                reply.send(turn, &Finished { output }).await?;
                turn.state_mut().started = None;
            }
            SandboxMsg::Stop { reply } => {
                self.containers.remove(&container).await?;
                reply.send(turn, &Stopped { container }).await?;
                let state = turn.state_mut();
                state.started = None;
                state.stopped = true;
            }
        }
        Ok(())
    }
}

/// An owner's notices, `example.notices/<owner>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Notices(pub String);

impl ActorRef for Notices {
    const TYPE: &'static str = "example.notices";
    type Message = NoticesMsg;
}

/// The messages of [`Notices`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum NoticesMsg {
    /// The job finished with `output`.
    Finished {
        /// The job.
        job: String,
        /// Its output.
        output: String,
    },
    /// The job was cancelled and cleaned up.
    Cancelled {
        /// The job.
        job: String,
    },
}

/// The notices an owner got, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoticesState {
    /// One line per notice: `finished <job> <output>` or `cancelled <job>`.
    #[serde(default)]
    pub lines: Vec<String>,
}

/// The notices actor.
pub struct NoticesActor;

impl Actor for NoticesActor {
    type Ref = Notices;
    type State = NoticesState;

    async fn handle(&self, turn: &mut Turn<Self>, message: NoticesMsg) -> HandlerResult {
        let line = match message {
            NoticesMsg::Finished { job, output } => format!("finished {job} {output}"),
            NoticesMsg::Cancelled { job } => format!("cancelled {job}"),
        };
        turn.state_mut().lines.push(line);
        Ok(())
    }
}

/// The input of a [`SandboxJob`] run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobInput {
    /// The job, which names its sandbox.
    pub job: String,
    /// The owner, whose notices get the report.
    pub owner: String,
}

/// The job flow, `example.sandbox-job`, calling `capacity`.
pub struct SandboxJob<C> {
    /// The capacity service.
    pub capacity: C,
}

/// The run of the job `key`, `example.sandbox-job/<key>`.
pub fn job<C: Capacity>(key: &str) -> FlowRun<SandboxJob<C>> {
    FlowRun::named(key)
}

impl<C: Capacity> Flow for SandboxJob<C> {
    const TYPE: &'static str = "example.sandbox-job";
    type Input = JobInput;

    fn retry() -> RetryPolicy {
        RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5)).expect("a valid backoff")).expect("a valid policy")
    }

    async fn run(&self, step: &mut Step, input: JobInput) -> HandlerResult {
        let capacity = &self.capacity;
        let reservation: String = step
            .run("reserve", |key| async move {
                capacity.reserve(&key).await?;
                Ok(key)
            })
            .await?;
        let sandbox = Sandbox(input.job.clone());
        let initial = SandboxState::default();
        let start =
            step.request("start", Target::create(&sandbox, &initial), |reply| SandboxMsg::Start { reply }).await?;
        let finished = step.wait("finished", &start).await?;
        step.run("release", |_key| async move { Ok(capacity.release(&reservation).await?) }).await?;
        let report = NoticesMsg::Finished { job: input.job, output: finished.output };
        step.send("report", &Notices(input.owner), report).await
    }

    async fn on_cancel(&self, cleanup: &mut Cleanup, input: JobInput) -> HandlerResult {
        let capacity = &self.capacity;
        // The reservation's key, also when the reservation happened and its result was never saved.
        let reservation = cleanup.main_key("reserve")?;
        if let Some(start) = cleanup.main_request::<Finished>("start")? {
            // Decided by the main journal alone: a reply that comes during the cleanup does not change it.
            if !matches!(cleanup.main_reply(&start)?, MainReply::Replied(_)) {
                let stop = |reply| SandboxMsg::Stop { reply };
                let stop = cleanup.request("stop", &Sandbox(input.job.clone()), stop).await?;
                cleanup.wait("stopped", &stop).await?;
            }
        }
        cleanup.run("release", |_key| async move { Ok(capacity.release(&reservation).await?) }).await?;
        cleanup.send("report", &Notices(input.owner), NoticesMsg::Cancelled { job: input.job }).await
    }
}
