//! Example 13 — the onboarding as a durable flow.
//!
//! The onboarding of [`steps`](crate::steps), against the same [`OnboardingServices`], written as ordinary async code:
//! a [`Flow`] whose steps create the account and send the welcome mail, then sleep a minute, durably, before they
//! provision a workspace. The [`FlowActor`](cicada_actors::FlowActor) runs each run as one actor,
//! `flows.onboarding/<user>`: every execution replays the results its journal holds and executes one new step, whose
//! result commits with the next `Continue`. So a committed step never runs again, and a failed step is retried by the
//! flow's policy (three attempts, five seconds apart). The sleep saves its wake time in the journal and commits with a
//! `Continue` due then; no handler waits meanwhile, and an early `Continue` replays it and waits for the same time. A
//! run that ran out of attempts keeps its saved steps; a public `Continue` resumes it there (see
//! `tests/flow_onboarding.rs`). Each call receives the key `flows.onboarding/<user>/<step id>`, the same on every
//! attempt. The flow wire of 0.18.0 changed every flow's contract (ADR 0044): the same test file runs this code under
//! a new type name next to a registration of 0.17 that an upgraded binary cannot host.
//!
//! A host registers the flow like any actor type, creates a run and sends it `Continue`:
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::{FlowActor, FlowMsg, FlowState};
//! # use cicada_examples::flow_onboarding::{OnboardingFlow, OnboardingInput, onboarding};
//! # use cicada_examples::steps::OnboardingServices;
//! # async fn run<S: OnboardingServices>(store: cicada::Store, services: S) -> Result<(), Box<dyn std::error::Error>> {
//! let system = ActorSystem::builder(store).host(FlowActor::new(OnboardingFlow { services })).build();
//! system.start().await?;
//! let run = onboarding::<S>("bob");
//! system.create(&run, &FlowState::new(OnboardingInput { email: "bob@example.test".into() })).await?;
//! system.send(&run, FlowMsg::Continue).await?;
//! # Ok(()) }
//! ```
//!
//! In a binary of the app layer it is one more type, as in the sample service:
//!
//! ```no_run
//! # use cicada_actors::FlowActor;
//! # use cicada_examples::flow_onboarding::OnboardingFlow;
//! # fn app(services: impl cicada_examples::steps::OnboardingServices) -> cicada_app::App {
//! cicada_app::App::new().host(FlowActor::new(OnboardingFlow { services }))
//! # }
//! ```

use std::time::Duration;

use cicada::prelude::*;
use cicada_actors::{Flow, FlowRun, Step};
use serde::{Deserialize, Serialize};

use crate::steps::OnboardingServices;

/// How long a run waits after the welcome mail before it provisions the workspace.
pub const COOL_OFF: Duration = Duration::from_secs(60);

/// The input of an onboarding run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OnboardingInput {
    /// The user's mail address.
    pub email: String,
}

/// The onboarding flow, `flows.onboarding`, calling `services`.
pub struct OnboardingFlow<S> {
    /// The external services.
    pub services: S,
}

/// The onboarding run of `user`, `flows.onboarding/<user>`.
pub fn onboarding<S: OnboardingServices>(user: &str) -> FlowRun<OnboardingFlow<S>> {
    FlowRun::named(user)
}

impl<S: OnboardingServices> Flow for OnboardingFlow<S> {
    const TYPE: &'static str = "flows.onboarding";
    type Input = OnboardingInput;

    fn retry() -> RetryPolicy {
        RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5)).expect("a valid backoff")).expect("a valid policy")
    }

    async fn run(&self, step: &mut Step, input: OnboardingInput) -> HandlerResult {
        let (services, email) = (&self.services, input.email.as_str());
        let account_id: String =
            step.run("create-account", |key| async move { Ok(services.create_account(email, &key).await?) }).await?;
        step.run("send-welcome", |key| async move { Ok(services.send_welcome(email, &key).await?) }).await?;
        step.sleep("cool-off", COOL_OFF).await?;
        let account_id = account_id.as_str();
        let _workspace_id: String = step
            .run("provision-workspace", |key| async move { Ok(services.provision_workspace(account_id, &key).await?) })
            .await?;
        Ok(())
    }
}
