//! The E2E actors on virtual time: they record the claim owner of each turn (`Turn::node_id`), not the node id their
//! constructors were given.

use cicada::runtime::NodeConfig;
use cicada::store::NodeId;
use cicada_sample_service::e2e::{
    Counter, CounterActor, CounterMsg, CounterState, EffectLog, Recorder, RecorderActor, RecorderMsg, RecorderState,
    Slow, SlowActor, SlowMsg, SlowState,
};
use cicada_test::TestActors;

/// sample-execution, Scenario: Existing constructor calls still compile.
#[tokio::test(start_paused = true)]
async fn the_e2e_actors_record_the_turn_owner_not_the_legacy_constructor_id() {
    let effects = EffectLog::new();
    let log = effects.clone();
    let test = TestActors::builder()
        .with(move |node| {
            node.host(SlowActor::new("legacy", log.clone()))
                .host(CounterActor::new("legacy"))
                .host(RecorderActor::new(String::from("legacy"), log))
                .config(NodeConfig { node_id: NodeId::new("runtime-1").unwrap(), ..NodeConfig::default() })
        })
        .build()
        .await
        .unwrap();
    let slow = Slow { id: "s".into() };
    let counter = Counter { id: "c".into() };
    let recorder = Recorder { id: "r".into() };
    test.create(&slow, &SlowState::default()).await;
    test.create(&counter, &CounterState::default()).await;
    test.create(&recorder, &RecorderState::default()).await;
    test.send(&slow, SlowMsg::Sleep { millis: 1_000, label: Some("nap".into()) }).await;
    test.send(&counter, CounterMsg::Step { seq: 1, hold_millis: 500 }).await;
    test.send(&recorder, RecorderMsg::Record { label: "effect".into(), sleep_millis: 0 }).await;
    assert_eq!(test.run_until_idle().await, 3);

    let runs = test.state::<_, SlowState>(&slow).await.runs;
    assert_eq!(runs.iter().map(|r| r.node.as_str()).collect::<Vec<_>>(), ["runtime-1"]);
    let entries = test.state::<_, CounterState>(&counter).await.entries;
    assert_eq!(entries.iter().map(|e| e.node.as_str()).collect::<Vec<_>>(), ["runtime-1"]);
    let records = test.state::<_, RecorderState>(&recorder).await.records;
    assert_eq!(records.iter().map(|r| r.node.as_str()).collect::<Vec<_>>(), ["runtime-1"]);
    let effects = effects.list(None);
    assert_eq!(effects.len(), 2, "{effects:?}");
    assert!(effects.iter().all(|e| e.node == "runtime-1"), "{effects:?}");
}
