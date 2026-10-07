//! Port of `OrderingTest` (T13): serial turns (G1), one outcome per message (G3) and mailbox order (G5) under load
//! from all four nodes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use cicada::api::MessageState;
use cicada_e2e::cluster::{cluster, eventually, run_id};
use cicada_e2e::ordering::{SentStep, assert_mailbox_order, send_step};
use cicada_e2e::{Class, step};
use cicada_sample_service::e2e::{Counter, CounterState};
use futures::future::join_all;

use crate::{millis, secs};

pub fn class() -> Class {
    Class::new(
        "OrderingTest",
        vec![
            step!(sequential_sends_through_rotating_nodes_are_handled_in_send_order),
            step!(concurrent_and_delayed_sends_through_all_nodes_keep_deliver_at_seq_order),
        ],
    )
}

/// Turns per node, for the log.
fn per_node(state: &CounterState) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for entry in &state.entries {
        *counts.entry(entry.node.as_str()).or_insert(0) += 1;
    }
    counts
}

async fn sequential_sends_through_rotating_nodes_are_handled_in_send_order() {
    let c = cluster();
    let counter = Counter { id: format!("seq-{}", run_id()) };
    c.node1.client.create(&counter, &CounterState::default()).await.unwrap();
    let n = 60;
    let mut sent = Vec::new();
    for seq in 1..=n {
        sent.push(send_step(&[c.nodes()[seq as usize % 4]], &counter, seq, 5, Duration::ZERO).await);
    }

    let state = eventually("all steps handled", secs(60), async || {
        let state: CounterState = c.node1.try_state(&counter).await?;
        Ok((state.count >= n).then_some(state))
    })
    .await;
    assert_eq!(state.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), (1..=n).collect::<Vec<_>>());
    assert_eq!(state.out_of_order, 0);
    assert_eq!(state.overlaps, 0);
    let mut statuses = HashMap::new();
    for s in &sent {
        statuses.insert(s.id.clone(), c.node1.await_finished(&s.id).await);
    }
    let outcomes: HashSet<MessageState> = statuses.values().map(|s| s.status).collect();
    assert_eq!(outcomes, HashSet::from([MessageState::Done]));
    assert_mailbox_order(&counter, &sent, &statuses, &state);
    println!("sequential: turns ran on {:?}", per_node(&state));
}

/// A small deterministic generator (xorshift64*) for the delays, seeded like the Kotlin test's `Random(42)`.
struct Random(u64);

impl Random {
    fn next_in(&mut self, from: u64, until: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        from + self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % (until - from)
    }
}

async fn concurrent_and_delayed_sends_through_all_nodes_keep_deliver_at_seq_order() {
    let c = cluster();
    let counters: Vec<Counter> = (1..=2).map(|i| Counter { id: format!("load-{}-{i}", run_id()) }).collect();
    for counter in &counters {
        c.node1.client.create(counter, &CounterState::default()).await.unwrap();
    }
    let per_sender = 50;
    let mut random = Random(42);
    let plan: Vec<Vec<(i64, Duration)>> = (0..4)
        .map(|sender| {
            (1..=per_sender)
                .map(|i| {
                    let seq = sender * 1000 + i;
                    // Every fifth step is delayed by 0.3-3 s, so delayed and immediate messages interleave in the mailbox.
                    let delay = if i % 5 == 0 { millis(random.next_in(300, 3000)) } else { Duration::ZERO };
                    (seq, delay)
                })
                .collect()
        })
        .collect();
    let senders = c.nodes().into_iter().zip(&plan).map(|(node, steps)| {
        let counters = &counters;
        async move {
            let mut sent = Vec::new();
            for (seq, delay) in steps {
                sent.push(send_step(&[node], &counters[(*seq % 2) as usize], *seq, 10, *delay).await);
            }
            sent
        }
    });
    let sent: Vec<SentStep> = join_all(senders).await.into_iter().flatten().collect();
    assert_eq!(sent.len(), 4 * per_sender as usize);

    let mut statuses = HashMap::new();
    for s in &sent {
        statuses.insert(s.id.clone(), c.node1.await_finished_within(&s.id, secs(120)).await);
    }
    let outcomes: HashSet<MessageState> = statuses.values().map(|s| s.status).collect();
    assert_eq!(outcomes, HashSet::from([MessageState::Done]));
    let attempts: HashSet<u32> = statuses.values().map(|s| s.message.attempt).collect();
    assert_eq!(attempts, HashSet::from([1]), "no turn was re-run without a fault");
    for counter in &counters {
        let state: CounterState = c.node1.state(counter).await;
        assert_mailbox_order(counter, &sent, &statuses, &state);
        println!("load {}: {} turns on {:?}", counter.id, state.count, per_node(&state));
    }
}
