//! Example 6 — durable steps.
//!
//! A multi-step job where every step is its own turn: the step runs, records its result in state and sends the next
//! step to self, all in one commit. A committed step is therefore never run again — a failure or crash in step 3 does
//! not repeat steps 1 and 2. A step that fails ends its message `failed` and leaves the state at that step; `Resume`
//! re-sends the current step (or send the steps with a retry policy to have failed steps retried automatically).

use std::fmt;
use std::future::Future;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

/// The onboarding of one user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Onboarding {
    /// The user.
    pub user: String,
}

impl ActorRef for Onboarding {
    const TYPE: &'static str = "onboarding";
    type Message = OnboardingMsg;
}

/// Messages of [`Onboarding`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum OnboardingMsg {
    /// Starts the onboarding for `email`.
    BeginOnboarding {
        /// The user's mail address.
        email: String,
    },
    /// Runs `step`; sent by the onboarding to itself.
    RunStep {
        /// The step to run.
        step: OnboardingStep,
    },
    /// Runs the current step again, after it failed.
    Resume,
}

/// The steps, in order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OnboardingStep {
    /// Not begun.
    #[default]
    NotStarted,
    /// Create the account.
    CreateAccount,
    /// Send the welcome mail.
    SendWelcome,
    /// Provision a workspace.
    ProvisionWorkspace,
    /// All steps done.
    Done,
}

impl OnboardingStep {
    /// The step after this one.
    pub fn next(self) -> Self {
        match self {
            Self::NotStarted => Self::CreateAccount,
            Self::CreateAccount => Self::SendWelcome,
            Self::SendWelcome => Self::ProvisionWorkspace,
            Self::ProvisionWorkspace | Self::Done => Self::Done,
        }
    }

    /// The step's name, as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            Self::NotStarted => "NOT_STARTED",
            Self::CreateAccount => "CREATE_ACCOUNT",
            Self::SendWelcome => "SEND_WELCOME",
            Self::ProvisionWorkspace => "PROVISION_WORKSPACE",
            Self::Done => "DONE",
        }
    }
}

impl fmt::Display for OnboardingStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// State of an [`Onboarding`]: the current step and what earlier steps produced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct OnboardingState {
    /// The current step.
    pub step: OnboardingStep,
    /// The user's mail address.
    pub email: Option<String>,
    /// The account, once created.
    pub account_id: Option<String>,
    /// The workspace, once provisioned.
    pub workspace_id: Option<String>,
}

/// A failed call of an external service.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ServiceError(pub String);

/// The external services the steps call. Each call is at least once (G4) and receives an idempotency key.
pub trait OnboardingServices: Send + Sync + 'static {
    /// Creates the account and returns its id.
    fn create_account(
        &self,
        email: &str,
        idempotency_key: &str,
    ) -> impl Future<Output = Result<String, ServiceError>> + Send;

    /// Sends the welcome mail.
    fn send_welcome(&self, email: &str, idempotency_key: &str)
    -> impl Future<Output = Result<(), ServiceError>> + Send;

    /// Provisions a workspace for `account_id` and returns its id.
    fn provision_workspace(
        &self,
        account_id: &str,
        idempotency_key: &str,
    ) -> impl Future<Output = Result<String, ServiceError>> + Send;
}

/// The onboarding type, calling `services`.
pub struct OnboardingActor<S> {
    /// The external services.
    pub services: S,
}

impl<S: OnboardingServices> Actor for OnboardingActor<S> {
    type Ref = Onboarding;
    type State = OnboardingState;

    async fn handle(&self, turn: &mut Turn<Self>, message: OnboardingMsg) -> HandlerResult {
        match message {
            OnboardingMsg::BeginOnboarding { email } => {
                if turn.state().step != OnboardingStep::NotStarted {
                    return Ok(());
                }
                let state = turn.state_mut();
                state.step = OnboardingStep::CreateAccount;
                state.email = Some(email);
                turn.send_self(OnboardingMsg::RunStep { step: OnboardingStep::CreateAccount })?;
            }
            OnboardingMsg::RunStep { step } => {
                if step != turn.state().step {
                    return Ok(()); // already done (or not yet due): never run a step twice
                }
                // A key per instance and step, so a Resume after a failure reuses it.
                let key = format!("{}/{step}", turn.self_address());
                let state = turn.state();
                let email = state.email.clone().ok_or_else(|| HandlerError::msg("a started onboarding has an email"));
                let account_id = state.account_id.clone().ok_or_else(|| HandlerError::msg("no account yet"));
                match step {
                    OnboardingStep::CreateAccount => {
                        let account_id = self.services.create_account(&email?, &key).await?;
                        turn.state_mut().account_id = Some(account_id);
                    }
                    OnboardingStep::SendWelcome => self.services.send_welcome(&email?, &key).await?,
                    OnboardingStep::ProvisionWorkspace => {
                        let workspace_id = self.services.provision_workspace(&account_id?, &key).await?;
                        turn.state_mut().workspace_id = Some(workspace_id);
                    }
                    OnboardingStep::NotStarted | OnboardingStep::Done => return Ok(()),
                }
                let next = step.next();
                turn.state_mut().step = next;
                if next != OnboardingStep::Done {
                    turn.send_self(OnboardingMsg::RunStep { step: next })?;
                }
            }
            OnboardingMsg::Resume => {
                let step = turn.state().step;
                if step != OnboardingStep::NotStarted && step != OnboardingStep::Done {
                    turn.send_self(OnboardingMsg::RunStep { step })?;
                }
            }
        }
        Ok(())
    }
}
