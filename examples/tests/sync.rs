//! The architecture's sync example.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada_examples::sync::{Page, Reports, ReportsActor, ReportsState, Sync, SyncActor, SyncApi, SyncMsg, SyncState};
use cicada_test::TestActors;

/// A fake external API: three pages of 10, 10 and 5 items.
#[derive(Clone, Default)]
struct FakeApi {
    keys: Arc<Mutex<Vec<String>>>,
}

impl SyncApi for FakeApi {
    async fn fetch(&self, _source: &str, cursor: Option<&str>, key: &str) -> Page {
        self.keys.lock().unwrap().push(key.to_owned());
        match cursor {
            None => Page { size: 10, next: Some("p2".into()) },
            Some("p2") => Page { size: 10, next: Some("p3".into()) },
            Some(_) => Page { size: 5, next: None },
        }
    }
}

async fn harness(api: &FakeApi) -> TestActors {
    let api = api.clone();
    TestActors::builder().with(move |node| node.host(SyncActor { api }).host(ReportsActor)).build().await.unwrap()
}

fn acme() -> Sync {
    Sync { tenant: "acme".into() }
}

fn reports() -> Reports {
    Reports { tenant: "acme".into() }
}

async fn set_up(test: &TestActors) {
    // The report target must exist: a send to an absent instance fails the turn.
    test.create(&reports(), &ReportsState::default()).await;
    test.create(&acme(), &SyncState { source: "https://api.acme.test/items".into(), imported: 0 }).await;
}

#[tokio::test(start_paused = true)]
async fn reschedules_itself() {
    let test = harness(&FakeApi::default()).await;
    set_up(&test).await;
    test.send(&acme(), SyncMsg::Run).await;
    test.run_until_idle().await; // every due message handled
    let next = test.pending(&acme()).await; // the delayed Run
    assert_eq!(next.len(), 1);
    assert_eq!(cicada::time::between(test.now(), next[0].deliver_at), Duration::from_secs(3600));
}

#[tokio::test(start_paused = true)]
async fn runs_hourly_each_page_its_own_turn_and_reports_every_run() {
    let api = FakeApi::default();
    let test = harness(&api).await;
    set_up(&test).await;
    test.send(&acme(), SyncMsg::Run).await;
    test.run_until_idle().await;
    assert_eq!(test.state::<_, SyncState>(&acme()).await.imported, 25);
    assert_eq!(api.keys.lock().unwrap().iter().collect::<BTreeSet<_>>().len(), 3); // one key per page message

    test.advance_time_by(Duration::from_secs(2 * 3600)).await;
    assert_eq!(test.state::<_, ReportsState>(&reports()).await.runs, vec![25, 25, 25]);
    assert!(test.failures().await.is_empty());
}
