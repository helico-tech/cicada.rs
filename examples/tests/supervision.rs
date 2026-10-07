//! Example 9: supervision and dead letters.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cicada_examples::supervision::{Mailer, MailerActor, MailerMsg, MailerState, Smtp, SmtpError};
use cicada_extras::{DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, FailureWatcher};
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
async fn mails_that_failed_while_the_server_was_down_are_redriven_from_the_dead_letters() {
    let smtp = FakeSmtp::default();
    let host = smtp.clone();
    let test = TestActors::builder()
        .with(move |node| node.host(MailerActor { smtp: host }).host(DeadLettersActor::default()))
        .build()
        .await
        .unwrap();
    let mut watcher = FailureWatcher::new(test.system().clone());
    let mailer = Mailer { id: "m".into() };
    test.create(&mailer, &MailerState::default()).await;
    let first = test.send(&mailer, MailerMsg::SendMail { to: "a@example.test".into() }).await;
    let second = test.send(&mailer, MailerMsg::SendMail { to: "b@example.test".into() }).await;
    test.run_until_idle().await;
    assert_eq!(watcher.poll().await.unwrap(), 2);
    test.run_until_idle().await;
    let letters: BTreeSet<String> = test.state::<_, DeadLettersState>(&DeadLetters).await.letters.into_keys().collect();
    assert_eq!(letters, BTreeSet::from([first.id.to_string(), second.id.to_string()]));

    smtp.up.store(true, Ordering::SeqCst);
    for failed_message_id in letters {
        test.send(&DeadLetters, DeadLettersMsg::Redrive { failed_message_id }).await;
    }
    test.run_until_idle().await;
    let sent: BTreeSet<String> = test.state::<_, MailerState>(&mailer).await.sent.into_iter().collect();
    assert_eq!(sent, BTreeSet::from(["a@example.test".to_owned(), "b@example.test".to_owned()]));
}
