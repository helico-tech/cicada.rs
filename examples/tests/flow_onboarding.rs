//! Example 13: the onboarding as a durable flow.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::store::OutcomeStatus;
use cicada_actors::{FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus};
use cicada_examples::flow_onboarding::{OnboardingFlow, OnboardingInput, onboarding};
use cicada_examples::steps::{OnboardingServices, ServiceError};
use cicada_test::TestActors;

/// Notes each call as `<service> <idempotency key>`; the workspace service fails while it is down.
#[derive(Clone, Default)]
struct FakeServices {
    calls: Arc<Mutex<Vec<String>>>,
    workspace_down: Arc<AtomicBool>,
}

impl FakeServices {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn note(&self, service: &str, key: &str) {
        self.calls.lock().unwrap().push(format!("{service} {key}"));
    }
}

impl OnboardingServices for FakeServices {
    async fn create_account(&self, _email: &str, idempotency_key: &str) -> Result<String, ServiceError> {
        self.note("account", idempotency_key);
        Ok("acc-1".into())
    }

    async fn send_welcome(&self, email: &str, idempotency_key: &str) -> Result<(), ServiceError> {
        assert_eq!(email, "bob@example.test");
        self.note("welcome", idempotency_key);
        Ok(())
    }

    async fn provision_workspace(&self, account_id: &str, idempotency_key: &str) -> Result<String, ServiceError> {
        assert_eq!(account_id, "acc-1", "the saved account id");
        self.note("workspace", idempotency_key);
        if self.workspace_down.load(Ordering::SeqCst) {
            return Err(ServiceError("workspace service unavailable".into()));
        }
        Ok("ws-1".into())
    }
}

type Run = FlowRun<OnboardingFlow<FakeServices>>;
type State = FlowState<OnboardingInput>;

fn bob() -> Run {
    onboarding("bob")
}

async fn harness(services: &FakeServices) -> TestActors {
    let services = services.clone();
    TestActors::builder()
        .with(move |node| node.host(FlowActor::new(OnboardingFlow { services })))
        .build()
        .await
        .unwrap()
}

async fn start(test: &TestActors) {
    test.create(&bob(), &FlowState::new(OnboardingInput { email: "bob@example.test".into() })).await;
    test.send(&bob(), FlowMsg::Continue).await;
}

#[tokio::test(start_paused = true)]
async fn each_step_commits_in_its_own_turn_until_the_run_is_done() {
    let services = FakeServices::default();
    let test = harness(&services).await;
    start(&test).await;
    assert_eq!(test.run_until_idle().await, 5, "the start, three steps and the completion");

    assert_eq!(
        services.calls(),
        [
            "account flows.onboarding/bob/create-account",
            "welcome flows.onboarding/bob/send-welcome",
            "workspace flows.onboarding/bob/provision-workspace",
        ]
    );
    let state: State = test.state(&bob()).await;
    assert_eq!((state.status, state.armed.clone()), (FlowStatus::Done, None));
    assert_eq!(state.result::<String>("create-account").unwrap().unwrap(), "acc-1");
    assert_eq!(state.result::<()>("send-welcome").map(|r| r.is_ok()), Some(true));
    assert_eq!(state.result::<String>("provision-workspace").unwrap().unwrap(), "ws-1");
    assert_eq!(test.version(&bob()).await, 5, "one commit per turn");
}

#[tokio::test(start_paused = true)]
async fn a_failing_step_is_retried_and_a_continue_resumes_it_without_repeating_the_committed_ones() {
    let services = FakeServices::default();
    services.workspace_down.store(true, Ordering::SeqCst);
    let test = harness(&services).await;
    start(&test).await;
    test.run_until_idle().await;
    test.advance_time_by(Duration::from_secs(10)).await;

    let state: State = test.state(&bob()).await;
    let failed = state.armed.clone().expect("the failed continuation stays armed");
    let outcome = test.system().outcome(&failed).await.unwrap().unwrap();
    assert_eq!((outcome.status, outcome.envelope.attempt), (OutcomeStatus::Failed, 3), "the flow's three attempts");
    assert_eq!(state.status, FlowStatus::Running);
    assert_eq!(state.journal.len(), 2, "the account and the welcome stay saved");

    services.workspace_down.store(false, Ordering::SeqCst);
    test.send(&bob(), FlowMsg::Continue).await;
    test.run_until_idle().await;
    let state: State = test.state(&bob()).await;
    assert_eq!(state.status, FlowStatus::Done);
    let calls = services.calls();
    assert_eq!(calls.iter().filter(|c| c.starts_with("account")).count(), 1, "{calls:?}");
    assert_eq!(calls.iter().filter(|c| c.starts_with("welcome")).count(), 1, "{calls:?}");
    let workspace: Vec<&String> = calls.iter().filter(|c| c.starts_with("workspace")).collect();
    assert_eq!(workspace.len(), 4, "three attempts and the resumed one");
    assert!(workspace.iter().all(|c| *c == "workspace flows.onboarding/bob/provision-workspace"), "one key: {calls:?}");
}
