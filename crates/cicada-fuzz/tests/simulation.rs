//! Node simulation: several real nodes under injected store faults, checked against G1, G3, G4, G5, G6 and S8.

use std::collections::BTreeMap;
use std::time::Duration;

use cicada::store::OutcomeStatus;
use cicada_fuzz::checks::check_all;
use cicada_fuzz::faults::{Ending, Event, Fault, HandlerEnd, Method, Rule};
use cicada_fuzz::simulation::{Action, PlannedFault, Scenario, Step, run, scenario, simulate};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

/// Set from measured time: about 4 ms a case in a debug build. `PROPTEST_CASES` overrides it.
const DEFAULT_CASES: u32 = 2_000;

fn config() -> Config {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|c| c.parse().ok()).unwrap_or(DEFAULT_CASES);
    Config { cases, ..Config::default() }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn guarantees_hold_under_faults(scenario in scenario()) {
        let events = simulate(&scenario);
        if let Err(violation) = check_all(&events) {
            return Err(TestCaseError::fail(violation.to_string()));
        }
    }
}

proptest! {
    #![proptest_config(Config { cases: 32, ..Config::default() })]

    #[test]
    fn a_scenario_replays_the_same(scenario in scenario()) {
        prop_assert_eq!(simulate(&scenario), simulate(&scenario));
    }
}

fn send(to: usize, actions: Vec<Action>) -> Step {
    Step::Send { node: 0, to, delay_ms: 0, retry: None, actions }
}

fn with_retry(to: usize, retry: u32, actions: Vec<Action>) -> Step {
    Step::Send { node: 0, to, delay_ms: 0, retry: Some(retry), actions }
}

fn ms(ms: u64) -> Step {
    Step::Advance(Duration::from_millis(ms))
}

fn sleep_action(ms: u64) -> Action {
    Action::Sleep { ms }
}

fn fault(node: usize, method: Method, nth: u32, fault: Fault) -> PlannedFault {
    PlannedFault { node, rule: Rule { method, nth, fault } }
}

fn outcome_of<'a>(events: &'a [Event], wanted: &str) -> Option<&'a OutcomeStatus> {
    events.iter().find_map(|e| match e {
        Event::Outcome { id, status } if id == wanted => Some(status),
        _ => None,
    })
}

#[tokio::test(start_paused = true)]
async fn each_script_action_happens() {
    let child = Action::Send { to: 1, delay_ms: 100, retry: None, actions: vec![Action::Create { to: 2 }] };
    let scenario = Scenario {
        nodes: 1,
        steps: vec![
            send(0, vec![child, sleep_action(50)]),
            with_retry(1, 2, vec![Action::Fail { times: 1 }]),
            with_retry(1, 2, vec![Action::Panic { times: 1 }]),
            send(2, vec![Action::Stop]),
        ],
        faults: Vec::new(),
    };
    let events = run(&scenario).await;
    check_all(&events).unwrap();
    assert!(
        events.contains(&Event::Accepted { id: "m0.0".to_owned(), to: "script/b".to_owned() }),
        "send: {events:#?}"
    );
    assert_eq!(outcome_of(&events, "m0.0"), Some(&OutcomeStatus::Done), "the child ran");
    assert!(
        events.iter().any(|e| matches!(e, Event::HandlerEnd { id, how: HandlerEnd::Failed, .. } if id == "m1")),
        "fail: {events:#?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, Event::HandlerEnd { id, how: HandlerEnd::Panicked, .. } if id == "m2")),
        "panic: {events:#?}"
    );
    assert_eq!(outcome_of(&events, "m1"), Some(&OutcomeStatus::Done), "the retry succeeded");
    assert_eq!(outcome_of(&events, "m2"), Some(&OutcomeStatus::Done), "the retry succeeded");
    assert!(events.contains(&Event::Removed { address: "script/c".to_owned() }), "stop: {events:#?}");
    assert!(
        events.contains(&Event::State { address: "script/c".to_owned(), ids: Vec::new() }),
        "create: the child made c again after the stop: {events:#?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_scenario_without_faults_drains_with_every_message_done() {
    let scenario = Scenario {
        nodes: 2,
        steps: vec![send(0, vec![sleep_action(5_000)]), ms(10), send(0, Vec::new()), send(1, vec![sleep_action(100)])],
        faults: Vec::new(),
    };
    let events = run(&scenario).await;
    check_all(&events).unwrap();
    for id in ["m0", "m1", "m2"] {
        assert_eq!(outcome_of(&events, id), Some(&OutcomeStatus::Done), "{id}: {events:#?}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_lost_acknowledgement_does_not_run_the_handler_again() {
    let scenario = Scenario {
        nodes: 1,
        steps: vec![send(0, Vec::new()), ms(1_000)],
        faults: vec![fault(0, Method::Commit, 0, Fault::After)],
    };
    let events = run(&scenario).await;
    check_all(&events).unwrap();
    let runs = events.iter().filter(|e| matches!(e, Event::HandlerStart { .. })).count();
    assert_eq!(runs, 1, "{events:#?}");
    assert_eq!(outcome_of(&events, "m0"), Some(&OutcomeStatus::Done));
}

#[tokio::test(start_paused = true)]
async fn the_work_of_a_dead_node_moves_to_another_node() {
    let scenario = Scenario {
        nodes: 2,
        steps: vec![ms(1_000), send(1, vec![sleep_action(2_000)]), ms(5_000)],
        faults: vec![
            fault(0, Method::ClaimDue, 0, Fault::Delay(Duration::from_secs(8))),
            fault(1, Method::Renew, 0, Fault::Dead),
        ],
    };
    let events = run(&scenario).await;
    check_all(&events).unwrap();
    assert_eq!(outcome_of(&events, "m0"), Some(&OutcomeStatus::Done), "{events:#?}");
}

/// Found by the first long run. A delete ends the instance while a handler still runs, and a new instance at that address
/// starts its own turn. Only the new one commits.
#[tokio::test(start_paused = true)]
async fn a_recreated_instance_is_a_new_instance() {
    let child = |actions| Action::Send { to: 2, delay_ms: 0, retry: None, actions };
    let scenario = Scenario {
        nodes: 1,
        steps: vec![
            send(0, vec![child(vec![sleep_action(1)])]),
            Step::Delete { node: 0, to: 2 },
            Step::Create { node: 0, to: 2 },
            send(0, vec![child(Vec::new())]),
        ],
        faults: Vec::new(),
    };
    let events = run(&scenario).await;
    check_all(&events).unwrap();
    assert_eq!(outcome_of(&events, "m0.0"), Some(&OutcomeStatus::Purged), "{events:#?}");
    assert_eq!(outcome_of(&events, "m1.0"), Some(&OutcomeStatus::Done), "{events:#?}");
}

#[test]
fn one_scenario_with_faults_replays_the_same() {
    let scenario = Scenario {
        nodes: 3,
        steps: vec![
            send(0, vec![sleep_action(4_000), Action::Send { to: 1, delay_ms: 0, retry: None, actions: Vec::new() }]),
            ms(500),
            with_retry(1, 3, vec![Action::Fail { times: 2 }]),
            ms(2_000),
            send(2, Vec::new()),
        ],
        faults: vec![
            fault(0, Method::Commit, 0, Fault::After),
            fault(1, Method::Renew, 1, Fault::Before),
            fault(2, Method::ClaimDue, 3, Fault::Delay(Duration::from_secs(4))),
            fault(1, Method::Commit, 1, Fault::Dead),
        ],
    };
    let first = simulate(&scenario);
    assert!(first.iter().any(|e| matches!(e, Event::Fault { .. })), "{first:#?}");
    assert_eq!(first, simulate(&scenario));
}

/// Generated scenarios must keep reaching the paths where bugs live. The runner is deterministic, so the counts only
/// change with the generators or the runtime.
#[test]
fn generated_scenarios_reach_deep_states() {
    let mut runner = TestRunner::deterministic();
    let mut marks: BTreeMap<&str, usize> = BTreeMap::new();
    for _ in 0..200 {
        let generated = scenario().new_tree(&mut runner).unwrap().current();
        let events = simulate(&generated);
        check_all(&events).unwrap();
        let mut mark = |name: &'static str, wanted: &dyn Fn(&Event) -> bool| {
            *marks.entry(name).or_insert(0) += events.iter().filter(|e| wanted(e)).count();
        };
        mark("accepted", &|e| matches!(e, Event::Accepted { .. }));
        mark("done", &|e| matches!(e, Event::Outcome { status: OutcomeStatus::Done, .. }));
        mark("failed", &|e| matches!(e, Event::Outcome { status: OutcomeStatus::Failed, .. }));
        mark("cancelled", &|e| matches!(e, Event::Outcome { status: OutcomeStatus::Cancelled, .. }));
        mark("purged", &|e| matches!(e, Event::Outcome { status: OutcomeStatus::Purged, .. }));
        mark("lost commit ack", &|e| matches!(e, Event::Fault { method: Method::Commit, fault: Fault::After, .. }));
        mark("delayed commit", &|e| matches!(e, Event::Fault { method: Method::Commit, fault: Fault::Delay(_), .. }));
        mark("failed renew", &|e| matches!(e, Event::Fault { method: Method::Renew, .. }));
        mark("dead", &|e| matches!(e, Event::Fault { fault: Fault::Dead, .. }));
        mark("commit refused", &|e| matches!(e, Event::CommitRefused { .. }));
        mark("reclaimed", &|e| matches!(e, Event::Claimed { attempt, .. } if *attempt > 1));
        mark("rescheduled", &|e| matches!(e, Event::Committed { ending: Ending::Rescheduled, .. }));
        mark("dropped", &|e| matches!(e, Event::HandlerEnd { how: HandlerEnd::Dropped, .. }));
        mark("panicked", &|e| matches!(e, Event::HandlerEnd { how: HandlerEnd::Panicked, .. }));
        mark("removed", &|e| matches!(e, Event::Removed { .. }));
    }
    let floors = [
        ("accepted", 1200),
        ("done", 900),
        ("failed", 150),
        ("cancelled", 15),
        ("purged", 100),
        ("lost commit ack", 25),
        ("delayed commit", 20),
        ("failed renew", 30),
        ("dead", 12),
        ("commit refused", 100),
        ("reclaimed", 60),
        ("rescheduled", 40),
        ("dropped", 20),
        ("panicked", 30),
        ("removed", 120),
    ];
    for (name, floor) in floors {
        assert!(marks[name] >= floor, "{name} reached only {} times, wanted {floor}: {marks:#?}", marks[name]);
    }
}
