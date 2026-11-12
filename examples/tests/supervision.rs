//! Example 9: supervision and dead letters.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cicada::{ActorRef, WatchTarget};
use cicada_actors::{
    Child, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, Supervisor, SupervisorActor, SupervisorMsg,
    SupervisorState,
};
use cicada_examples::supervision::{Mailer, MailerActor, MailerMsg, MailerState, Smtp, SmtpError};
use cicada_test::TestActors;

/// A mail server that is down until `up` is set.
#[derive(Clone, Default)]
struct FakeSmtp {
    up: Arc<AtomicBool>,
}

impl Smtp for FakeSmtp {
    async fn send(&self, _to: &str) -> Result<(), SmtpError> {
        if self.up.load(Ordering::SeqCst) { Ok(()) } else { Err(SmtpError("smtp down".into())) }
    }
}

#[tokio::test(start_paused = true)]
async fn a_mail_that_failed_while_the_server_was_down_restarts_the_mailer_and_is_redriven_from_the_dead_letters() {
    let smtp = FakeSmtp::default();
    let host = smtp.clone();
    let test = TestActors::builder()
        .with(move |node| node.host(MailerActor { smtp: host }).host(DeadLettersActor::default()).host(SupervisorActor))
        .build()
        .await
        .unwrap();
    let letters = DeadLetters::default();
    test.create(&letters, &DeadLettersState::default()).await;
    test.system().watch(&WatchTarget::Every, &letters.address().unwrap()).await.unwrap();
    let mailer = Mailer { id: "m".into() };
    let supervisor = Supervisor::named("mailers");
    let children = SupervisorState::new().child(Child::new(&mailer, &MailerState::default()).unwrap());
    test.create(&supervisor, &children).await;
    test.send(&supervisor, SupervisorMsg::Start).await;
    test.run_until_idle().await;

    let failed = test.send(&mailer, MailerMsg::SendMail { to: "a@example.test".into() }).await;
    test.run_until_idle().await;
    let state: SupervisorState = test.state(&supervisor).await;
    assert_eq!(state.children[&mailer.address().unwrap()].restarts.len(), 1, "the supervisor restarted the mailer");
    let recorded = test.state::<_, DeadLettersState>(&letters).await.letters;
    assert_eq!(recorded.keys().collect::<Vec<_>>(), [&failed.id], "the failed mail is a dead letter");
    assert_eq!(recorded[&failed.id].error, "smtp down");

    smtp.up.store(true, Ordering::SeqCst);
    test.send(&letters, DeadLettersMsg::Redrive { message_id: failed.id.clone() }).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, MailerState>(&mailer).await.sent, ["a@example.test"]);
}
