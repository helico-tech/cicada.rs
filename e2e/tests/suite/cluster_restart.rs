//! The whole cluster (PostgreSQL included) restarts with delayed
//! messages pending.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use cicada::api::MessageState;
use cicada_e2e::cluster::{cluster, eventually, run_id};
use cicada_e2e::docker::compose;
use cicada_e2e::ordering::{assert_mailbox_order, send_step};
use cicada_e2e::{Class, step};
use cicada_sample_service::e2e::{Counter, CounterState, Slow, SlowMsg, SlowState};

use crate::node_fault::restore;
use crate::secs;

pub fn class() -> Class {
    Class {
        after_each: Some(step!(restore).1),
        ..Class::new(
            "ClusterRestartTest",
            vec![step!(pending_delayed_messages_are_delivered_after_a_restart_of_the_whole_cluster)],
        )
    }
}

async fn pending_delayed_messages_are_delivered_after_a_restart_of_the_whole_cluster() {
    let c = cluster();
    let counter = Counter { id: format!("restart-{}", run_id()) };
    c.node1.client.create(&counter, &CounterState::default()).await.unwrap();
    let before = send_step(&c.nodes(), &counter, 1, 0, Duration::ZERO).await;
    assert_eq!(c.node1.await_finished(&before.id).await.status, MessageState::Done);

    // Due after the restart (a restart takes ~15-25 s); different delays, sent out of order.
    let mut delayed = Vec::new();
    for (seq, seconds) in [(4, 24), (2, 18), (3, 21), (5, 27)] {
        delayed.push(send_step(&[c.nodes()[seq as usize % 4]], &counter, seq, 0, secs(seconds)).await);
    }
    // One in the middle of a turn as well: released by the graceful stop, re-run after the restart.
    let slow = Slow { id: format!("restart-{}", run_id()) };
    c.node3.client.create(&slow, &SlowState::default()).await.unwrap();
    let message = SlowMsg::Sleep { millis: 8_000, label: Some("restart".to_owned()) };
    let in_flight = c.send_only.client.send(&slow, message).await.unwrap();
    eventually("slow turn running", secs(30), async || {
        Ok((c.send_only.try_status(&in_flight.id).await?.status == MessageState::Running).then_some(()))
    })
    .await;

    let restart = Instant::now();
    compose::restart().await.unwrap();
    c.await_healthy(secs(90)).await;
    println!("restart: cluster healthy again after {:.1?}", restart.elapsed());

    let sent: Vec<_> = std::iter::once(before).chain(delayed).collect();
    let mut statuses = HashMap::new();
    for s in &sent {
        statuses.insert(s.id.clone(), c.send_only.await_finished_within(&s.id, secs(90)).await);
    }
    let outcomes: HashSet<MessageState> = statuses.values().map(|s| s.status).collect();
    assert_eq!(outcomes, HashSet::from([MessageState::Done]));
    let state: CounterState = c.send_only.state(&counter).await;
    assert_mailbox_order(&counter, &sent, &statuses, &state);
    assert_eq!(state.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(c.send_only.view(&counter).await.version, 5, "state written before the restart survived");

    let slow_done = c.send_only.await_finished_within(&in_flight.id, secs(60)).await;
    assert_eq!(slow_done.status, MessageState::Done);
    let runs = c.send_only.state::<SlowState, _>(&slow).await.runs;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].attempt >= 1);
    println!("restart: in-flight turn committed with attempt {} on {}", runs[0].attempt, runs[0].node);
}
