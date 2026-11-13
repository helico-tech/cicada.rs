//! Example 10: a shared rate limit.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::NoState;
use cicada_actors::{RateLimiterActor, RateLimiterState};
use cicada_examples::rate_limit::{
    Gateway, GatewayError, Signup, SignupActor, SignupMsg, SignupState, Sms, SmsActor, sms_limiter,
};
use cicada_test::TestActors;

/// A gateway that keeps the phone numbers it texted, and fails the first send to `flaky`.
#[derive(Clone, Default)]
struct FakeGateway {
    sent: Arc<Mutex<Vec<String>>>,
    failed_once: Arc<Mutex<bool>>,
}

impl Gateway for FakeGateway {
    async fn send(&self, phone: &str, _text: &str) -> Result<(), GatewayError> {
        if phone == "flaky" && !std::mem::replace(&mut *self.failed_once.lock().unwrap(), true) {
            return Err(GatewayError("gateway down".into()));
        }
        self.sent.lock().unwrap().push(phone.to_owned());
        Ok(())
    }
}

async fn node(gateway: FakeGateway) -> TestActors {
    let test = TestActors::builder()
        .with(move |node| node.host(RateLimiterActor).host(SignupActor).host(SmsActor { gateway }))
        .build()
        .await
        .unwrap();
    test.create(&Sms, &NoState {}).await;
    test
}

fn signup(n: usize) -> Signup {
    Signup { id: n.to_string() }
}

#[tokio::test(start_paused = true)]
async fn seven_signups_at_once_text_five_now_and_one_a_second_after() {
    let gateway = FakeGateway::default();
    let test = node(gateway.clone()).await;
    for n in 0..7 {
        test.create(&signup(n), &SignupState::default()).await;
        test.send(&signup(n), SignupMsg::Register { phone: format!("+31 6 {n}") }).await;
    }
    test.run_until_idle().await;
    assert_eq!(gateway.sent.lock().unwrap().len(), 5, "the burst");
    test.advance_time_by(Duration::from_secs(1)).await;
    assert_eq!(gateway.sent.lock().unwrap().len(), 6);
    test.advance_time_by(Duration::from_secs(1)).await;
    assert_eq!(gateway.sent.lock().unwrap().len(), 7);
    for n in 0..7 {
        assert!(test.state::<_, SignupState>(&signup(n)).await.texted, "the reply reached signup {n}");
    }
    let limiter: RateLimiterState = test.state(&sms_limiter()).await;
    assert_eq!((limiter.count, limiter.burst), (1, 5), "created by the first signup");
}

#[tokio::test(start_paused = true)]
async fn a_text_that_fails_at_the_gateway_is_tried_again_by_the_policy_of_the_submission() {
    let gateway = FakeGateway::default();
    let test = node(gateway.clone()).await;
    test.create(&signup(1), &SignupState::default()).await;
    test.send(&signup(1), SignupMsg::Register { phone: "flaky".into() }).await;
    test.run_until_idle().await;
    assert!(gateway.sent.lock().unwrap().is_empty(), "the first send failed");
    let reserved = test.state::<_, RateLimiterState>(&sms_limiter()).await;
    test.advance_time_by(Duration::from_secs(5)).await;
    assert_eq!(*gateway.sent.lock().unwrap(), ["flaky"]);
    assert!(test.state::<_, SignupState>(&signup(1)).await.texted);
    assert_eq!(test.state::<_, RateLimiterState>(&sms_limiter()).await, reserved, "the retry did not pass the limiter");
}
