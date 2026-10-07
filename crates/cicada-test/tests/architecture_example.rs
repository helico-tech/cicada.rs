//! The architecture's sync example, run end to end with virtual time.

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sync {
    tenant: String,
}

impl ActorRef for Sync {
    const TEMPLATE: &'static str = "actor://sync/{tenant}";
    type Message = SyncMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SyncMsg {
    Run,
    FetchPage { cursor: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SyncState {
    source: String,
    #[serde(default)]
    imported: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Reports {
    tenant: String,
}

impl ActorRef for Reports {
    const TEMPLATE: &'static str = "actor://reports/{tenant}";
    type Message = ReportsMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ReportsMsg {
    SyncFinished { imported: u32 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
struct ReportsState {
    #[serde(default)]
    runs: Vec<u32>,
}

/// A fake paged API: three pages of two items.
struct SyncActor;

impl Actor for SyncActor {
    type Ref = Sync;
    type State = SyncState;
    type Message = SyncMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(300));

    async fn handle(&self, turn: &mut Turn<Self>, message: SyncMsg) -> HandlerResult {
        match message {
            SyncMsg::Run => {
                turn.state_mut().imported = 0;
                turn.send_self(SyncMsg::FetchPage { cursor: None })?;
            }
            SyncMsg::FetchPage { cursor } => {
                let page: u32 = cursor.as_deref().map(|c| c.parse().unwrap()).unwrap_or(0);
                turn.state_mut().imported += 2;
                if page < 2 {
                    turn.send_self(SyncMsg::FetchPage { cursor: Some((page + 1).to_string()) })?;
                } else {
                    let tenant = turn.self_ref().tenant.clone();
                    let imported = turn.state().imported;
                    turn.send(&Reports { tenant }, ReportsMsg::SyncFinished { imported })?;
                    turn.send_self_with(SyncMsg::Run, SendOptions::new().delay(Duration::from_secs(3600)))?;
                }
            }
        }
        Ok(())
    }
}

struct ReportsActor;

impl Actor for ReportsActor {
    type Ref = Reports;
    type State = ReportsState;
    type Message = ReportsMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: ReportsMsg) -> HandlerResult {
        let ReportsMsg::SyncFinished { imported } = message;
        turn.state_mut().runs.push(imported);
        Ok(())
    }
}

async fn harness() -> TestActors {
    TestActors::builder().with(|node| node.host(SyncActor).host(ReportsActor)).build().await.unwrap()
}

#[tokio::test(start_paused = true)]
async fn reschedules_itself() {
    let test = harness().await;
    let sync = Sync { tenant: "acme".into() };
    let reports = Reports { tenant: "acme".into() };
    test.create(&reports, &ReportsState::default()).await;
    test.create(&sync, &SyncState { source: "https://api.acme.test/items".into(), imported: 0 }).await;
    let run = test.send(&sync, SyncMsg::Run).await;

    assert_eq!(test.run_until_idle().await, 5);
    let next = test.pending(&sync).await;
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].message_type, "Run");
    assert_eq!(cicada::time::between(test.now(), next[0].deliver_at), Duration::from_secs(3600));
    assert_eq!(test.state::<_, ReportsState>(&reports).await.runs, vec![6]);
    assert_eq!(test.outcome(&run).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, SyncState>(&sync).await.imported, 6);
    assert_eq!(test.version(&sync).await, 4);

    // Two hours later the hourly run happened twice more.
    assert_eq!(test.advance_time_by(Duration::from_secs(7200)).await, 10);
    assert_eq!(test.state::<_, ReportsState>(&reports).await.runs, vec![6, 6, 6]);
}

#[tokio::test(start_paused = true)]
async fn a_send_to_an_absent_instance_fails_the_turn() {
    let test = harness().await;
    let sync = Sync { tenant: "acme".into() };
    test.create(&sync, &SyncState { source: "s".into(), imported: 0 }).await;
    test.send(&sync, SyncMsg::Run).await;
    test.run_until_idle().await;
    let failures = test.failures().await;
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].envelope.message_type, "FetchPage");
    assert!(failures[0].error.as_deref().unwrap().contains("Send to absent instance actor://reports/acme"));
    // The failed turn's effects are not applied.
    assert!(test.pending(&sync).await.is_empty());
}
