//! Example 12: batching audit events into bulk writes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::NoState;
use cicada_actors::{BatcherActor, BatcherMsg, BatcherState};
use cicada_examples::batching::{
    Account, AccountActor, AccountMsg, AccountState, AuditError, AuditEvent, AuditLog, AuditLogActor, AuditStore,
    audit_batcher,
};
use cicada_test::TestActors;

const SECOND: Duration = Duration::from_secs(1);

/// An audit store that keeps each write as the actions it held, and fails the first write that holds `flaky`.
#[derive(Clone, Default)]
struct FakeAudit {
    writes: Arc<Mutex<Vec<Vec<String>>>>,
    failed_once: Arc<Mutex<bool>>,
}

impl AuditStore for FakeAudit {
    async fn write_all(&self, events: &[AuditEvent]) -> Result<(), AuditError> {
        let flaky = events.iter().any(|e| e.action.ends_with("flaky"));
        if flaky && !std::mem::replace(&mut *self.failed_once.lock().unwrap(), true) {
            return Err(AuditError("audit store down".into()));
        }
        self.writes.lock().unwrap().push(events.iter().map(|e| e.action.clone()).collect());
        Ok(())
    }
}

impl FakeAudit {
    fn writes(&self) -> Vec<Vec<String>> {
        self.writes.lock().unwrap().clone()
    }
}

/// A node with the batcher, the accounts and, unless `audit_log` is false, the audit log.
async fn node(audit: FakeAudit, audit_log: bool) -> TestActors {
    let test = TestActors::builder()
        .with(move |node| node.host(BatcherActor).host(AccountActor).host(AuditLogActor { audit }))
        .build()
        .await
        .unwrap();
    if audit_log {
        test.create(&AuditLog, &NoState {}).await;
    }
    test
}

fn account(id: usize) -> Account {
    Account { id: format!("a{id}") }
}

async fn rename(test: &TestActors, id: usize, name: &str) {
    let account = account(id);
    if !test.exists(&account).await {
        test.create(&account, &AccountState::default()).await;
    }
    test.send(&account, AccountMsg::Rename { name: name.into() }).await;
    test.run_until_idle().await;
}

async fn held(test: &TestActors) -> BatcherState {
    test.state(&audit_batcher()).await
}

#[tokio::test(start_paused = true)]
async fn fifty_events_are_written_at_once_in_one_call() {
    let audit = FakeAudit::default();
    let test = node(audit.clone(), true).await;
    for n in 0..51 {
        rename(&test, n % 3, &format!("n{n}")).await;
    }
    let writes = audit.writes();
    assert_eq!(writes.len(), 1, "the fiftieth event filled the batch");
    assert_eq!(
        (writes[0].len(), writes[0][0].as_str(), writes[0][49].as_str()),
        (50, "renamed to n0", "renamed to n49")
    );
    assert_eq!(held(&test).await.items.len(), 1, "the 51st event starts the next batch");
}

#[tokio::test(start_paused = true)]
async fn a_few_events_are_written_five_seconds_after_the_first() {
    let audit = FakeAudit::default();
    let test = node(audit.clone(), true).await;
    rename(&test, 1, "x").await;
    test.advance_time_by(2 * SECOND).await;
    rename(&test, 2, "y").await;
    test.advance_time_by(2 * SECOND).await;
    assert!(audit.writes().is_empty(), "within the longest wait");
    test.advance_time_by(SECOND).await;
    assert_eq!(audit.writes(), [["renamed to x", "renamed to y"]]);
    assert!(held(&test).await.items.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_write_that_fails_is_tried_again_by_the_policy_of_the_settings() {
    let audit = FakeAudit::default();
    let test = node(audit.clone(), true).await;
    rename(&test, 1, "flaky").await;
    test.advance_time_by(5 * SECOND).await;
    assert!(audit.writes().is_empty(), "the first write failed");
    assert!(held(&test).await.items.is_empty(), "flushed");
    test.advance_time_by(5 * SECOND).await;
    assert_eq!(audit.writes(), [["renamed to flaky"]]);
}

#[tokio::test(start_paused = true)]
async fn a_flush_to_a_missing_audit_log_keeps_the_events_and_an_operator_flushes_them_later() {
    let audit = FakeAudit::default();
    let test = node(audit.clone(), false).await;
    rename(&test, 1, "x").await;
    rename(&test, 2, "y").await;
    test.advance_time_by(5 * SECOND).await;
    let failures = test.failures().await;
    assert!(failures.iter().any(|f| f.envelope.message_type == "Flush"), "{failures:?}");
    assert_eq!(held(&test).await.items.len(), 2, "the events stay");
    test.create(&AuditLog, &NoState {}).await;
    test.send(&audit_batcher(), BatcherMsg::Flush).await;
    test.run_until_idle().await;
    assert_eq!(audit.writes(), [["renamed to x", "renamed to y"]]);
}
