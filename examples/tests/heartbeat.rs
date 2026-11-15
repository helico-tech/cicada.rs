//! Example 19: noticing a nightly import that did not run.

use std::time::Duration;

use cicada::time::Timestamp;
use cicada_actors::{HeartbeatStatus, HeartbeatWatchActor, HeartbeatWatchState};
use cicada_examples::heartbeat::{
    ALLOWED_SILENCE, Import, ImportActor, ImportMsg, ImportState, OnCall, OnCallActor, OnCallState, import_watch,
};
use cicada_test::TestActors;

const HOUR: Duration = Duration::from_secs(3600);

/// A node at midnight with the watch type, the on-call desk and the import of `acme`, started.
async fn node() -> TestActors {
    let test = TestActors::builder()
        .start_at("2026-03-01T00:00:00Z".parse().unwrap())
        .with(|node| node.host(HeartbeatWatchActor).host(ImportActor).host(OnCallActor))
        .build()
        .await
        .unwrap();
    test.create(&OnCall, &OnCallState::default()).await;
    test.create(&acme(), &ImportState::default()).await;
    test.send(&acme(), ImportMsg::Start).await;
    test.run_until_idle().await;
    test
}

fn acme() -> Import {
    Import { partner: "acme".into() }
}

async fn file(test: &TestActors, name: &str) {
    test.send(&acme(), ImportMsg::FileImported { file: name.into() }).await;
    test.run_until_idle().await;
}

async fn alerts(test: &TestActors) -> Vec<String> {
    test.state::<_, OnCallState>(&OnCall).await.alerts
}

async fn watch(test: &TestActors) -> HeartbeatWatchState {
    test.state(&import_watch("acme")).await
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_file_every_night_keeps_the_watch_quiet() {
    let test = node().await;
    for night in 1..=5 {
        test.advance_time_by(24 * HOUR).await;
        file(&test, &format!("night-{night}.csv")).await;
    }
    test.advance_time_by(24 * HOUR).await;
    assert!(alerts(&test).await.is_empty());
    assert_eq!(test.state::<_, ImportState>(&acme()).await.files.len(), 5);
    assert_eq!(watch(&test).await.status, HeartbeatStatus::Alive);
}

#[tokio::test(start_paused = true)]
async fn an_import_that_never_gets_a_file_is_noticed_after_25_hours() {
    let test = node().await;
    assert_eq!(watch(&test).await.status, HeartbeatStatus::Alive, "the first Beat came with the create");
    test.advance_time_by(ALLOWED_SILENCE - Duration::from_secs(1)).await;
    assert!(alerts(&test).await.is_empty());
    test.advance_time_by(Duration::from_secs(1)).await;
    assert_eq!(alerts(&test).await, ["acme import silent at 2026-03-02T01:00:00Z"]);
}

#[tokio::test(start_paused = true)]
async fn a_missed_night_raises_one_alert_and_the_next_file_recovers_without_a_message() {
    let test = node().await;
    test.advance_time_by(24 * HOUR).await;
    file(&test, "night-1.csv").await;
    test.advance_time_by(4 * 24 * HOUR).await;
    assert_eq!(alerts(&test).await, ["acme import silent at 2026-03-03T01:00:00Z"], "one silence, one alert");
    let silent = watch(&test).await;
    assert_eq!((silent.status, silent.last_beat_at), (HeartbeatStatus::Silent, Some(at("2026-03-02T00:00:00Z"))));

    file(&test, "night-5.csv").await;
    let alive = watch(&test).await;
    assert_eq!((alive.status, alive.last_beat_at), (HeartbeatStatus::Alive, Some(at("2026-03-06T00:00:00Z"))));
    assert_eq!(alerts(&test).await.len(), 1, "the recovery tells nobody");
    test.advance_time_by(ALLOWED_SILENCE).await;
    assert_eq!(alerts(&test).await.len(), 2, "the next silence alerts again");
}
