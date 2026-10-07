//! Example 6: durable steps.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cicada::store::OutcomeStatus;
use cicada_examples::steps::{
    Onboarding, OnboardingActor, OnboardingMsg, OnboardingServices, OnboardingState, OnboardingStep, ServiceError,
};
use cicada_test::TestActors;

#[derive(Clone, Default)]
struct FakeServices {
    calls: Arc<Mutex<Vec<&'static str>>>,
    workspace_down: Arc<AtomicBool>,
}

impl FakeServices {
    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

impl OnboardingServices for FakeServices {
    async fn create_account(&self, _email: &str, _idempotency_key: &str) -> Result<String, ServiceError> {
        self.calls.lock().unwrap().push("account");
        Ok("acc-1".into())
    }

    async fn send_welcome(&self, _email: &str, _idempotency_key: &str) -> Result<(), ServiceError> {
        self.calls.lock().unwrap().push("welcome");
        Ok(())
    }

    async fn provision_workspace(&self, _account_id: &str, _idempotency_key: &str) -> Result<String, ServiceError> {
        self.calls.lock().unwrap().push("workspace");
        if self.workspace_down.load(Ordering::SeqCst) {
            return Err(ServiceError("workspace service unavailable".into()));
        }
        Ok("ws-1".into())
    }
}

fn bob() -> Onboarding {
    Onboarding { user: "bob".into() }
}

async fn harness(services: &FakeServices) -> TestActors {
    let services = services.clone();
    TestActors::builder().with(move |node| node.host(OnboardingActor { services })).build().await.unwrap()
}

fn begin() -> OnboardingMsg {
    OnboardingMsg::BeginOnboarding { email: "bob@example.test".into() }
}

#[tokio::test(start_paused = true)]
async fn each_step_runs_in_its_own_turn_until_done() {
    let services = FakeServices::default();
    let test = harness(&services).await;
    test.create(&bob(), &OnboardingState::default()).await;
    test.send(&bob(), begin()).await;
    assert_eq!(test.run_until_idle().await, 4); // begin + three steps

    assert_eq!(services.calls(), vec!["account", "welcome", "workspace"]);
    let expected = OnboardingState {
        step: OnboardingStep::Done,
        email: Some("bob@example.test".into()),
        account_id: Some("acc-1".into()),
        workspace_id: Some("ws-1".into()),
    };
    assert_eq!(test.state::<_, OnboardingState>(&bob()).await, expected);
    assert_eq!(test.version(&bob()).await, 4); // one commit per step
}

#[tokio::test(start_paused = true)]
async fn a_failing_step_does_not_repeat_the_committed_ones_and_resume_continues_there() {
    let services = FakeServices::default();
    services.workspace_down.store(true, Ordering::SeqCst);
    let test = harness(&services).await;
    test.create(&bob(), &OnboardingState::default()).await;
    test.send(&bob(), begin()).await;
    test.run_until_idle().await;

    assert_eq!(test.state::<_, OnboardingState>(&bob()).await.step, OnboardingStep::ProvisionWorkspace);
    let failures = test.failures().await;
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].envelope.message_type, "RunStep");
    assert_eq!(failures[0].status, OutcomeStatus::Failed);

    services.workspace_down.store(false, Ordering::SeqCst);
    test.send(&bob(), OnboardingMsg::Resume).await;
    test.send(&bob(), OnboardingMsg::Resume).await; // the second Resume's step is stale by the time it runs
    test.run_until_idle().await;

    assert_eq!(test.state::<_, OnboardingState>(&bob()).await.step, OnboardingStep::Done);
    assert_eq!(services.calls(), vec!["account", "welcome", "workspace", "workspace"]);
}
