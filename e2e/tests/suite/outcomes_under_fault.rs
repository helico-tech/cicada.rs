//! N messages to M actors through all nodes while a node is killed and
//! started again in the middle. Every accepted message ends with exactly one outcome (`done`), each committed exactly
//! once, in mailbox order.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cicada::api::MessageState;
use cicada_e2e::cluster::{Node, cluster, eventually, run_id};
use cicada_e2e::docker::compose;
use cicada_e2e::ordering::{SentStep, assert_mailbox_order, send_step};
use cicada_e2e::{Class, step};
use cicada_sample_service::e2e::{Counter, CounterState};
use futures::future::join_all;

use crate::node_fault::restore;
use crate::{millis, secs};

pub fn class() -> Class {
    Class {
        after_each: Some(step!(restore).1),
        ..Class::new(
            "OutcomesUnderFaultTest",
            vec![step!(no_lost_or_duplicated_outcomes_while_a_node_is_killed_and_restarted)],
        )
    }
}

async fn no_lost_or_duplicated_outcomes_while_a_node_is_killed_and_restarted() {
    let c = cluster();
    let counters: Vec<Counter> = (1..=4).map(|i| Counter { id: format!("g3-{}-{i}", run_id()) }).collect();
    for counter in &counters {
        c.node1.client.create(counter, &CounterState::default()).await.unwrap();
    }
    let per_sender = 40;
    let sent_count = AtomicUsize::new(0);
    let victim = &c.node2;

    let senders = c.nodes().into_iter().enumerate().map(|(index, first)| {
        // Each sender prefers its own node and fails over (same id) to the others.
        let order: Vec<&Node> =
            std::iter::once(first).chain(c.nodes().into_iter().filter(|n| n.id != first.id)).collect();
        let (counters, sent_count) = (&counters, &sent_count);
        async move {
            let mut sent = Vec::new();
            for i in 1..=per_sender {
                let seq = index as i64 * 1000 + i as i64;
                sent.push(send_step(&order, &counters[i % counters.len()], seq, 50, Duration::ZERO).await);
                sent_count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(millis(20)).await;
            }
            sent
        }
    });
    let chaos = async {
        eventually("a quarter sent", secs(30), async || {
            Ok((sent_count.load(Ordering::SeqCst) >= per_sender).then_some(()))
        })
        .await;
        compose::kill(&victim.id).await.unwrap();
        println!("g3: {victim} killed after {} sends", sent_count.load(Ordering::SeqCst));
        tokio::time::sleep(secs(6)).await;
        compose::start(&victim.id, true).await.unwrap();
        eventually(&format!("{victim} back"), secs(60), async || Ok(victim.healthy().await.then_some(()))).await;
        println!("g3: {victim} back after {} sends", sent_count.load(Ordering::SeqCst));
    };
    let (sent, ()) = tokio::join!(join_all(senders), chaos);
    let sent: Vec<SentStep> = sent.into_iter().flatten().collect();
    assert_eq!(sent.len(), 4 * per_sender);
    assert_eq!(sent.iter().map(|s| &s.id).collect::<HashSet<_>>().len(), sent.len());

    let mut statuses = HashMap::new();
    for s in &sent {
        statuses.insert(s.id.clone(), c.send_only.await_finished_within(&s.id, secs(120)).await);
    }
    let mut by_status = BTreeMap::new();
    for status in statuses.values() {
        *by_status.entry(format!("{:?}", status.status)).or_insert(0) += 1;
    }
    assert_eq!(
        by_status,
        BTreeMap::from([(format!("{:?}", MessageState::Done), sent.len())]),
        "every accepted message is done exactly once"
    );
    let mut total = 0;
    for counter in &counters {
        let state: CounterState = c.send_only.state(counter).await;
        assert_mailbox_order(counter, &sent, &statuses, &state);
        total += state.count;
    }
    assert_eq!(total, sent.len() as i64, "done count == N");
    let re_run = statuses.values().filter(|s| s.message.attempt > 1).count();
    println!("g3: {} messages, {re_run} re-run after the kill", sent.len());
    assert!(statuses.values().all(|s| s.message.attempt <= 2));
    assert!(c.send_only.failures(&format!("e2e.counter/g3-{}-", run_id())).await.is_empty());
}
