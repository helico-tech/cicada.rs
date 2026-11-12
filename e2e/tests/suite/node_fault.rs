//! A node killed, paused or stopped in the middle of a turn. All observation is over
//! HTTP through the send-only node (never the victim); every test puts the cluster back to all-up afterwards.

use std::collections::HashSet;
use std::time::Instant;

use cicada::api::{MessageState, MessageStatusView};
use cicada::store::MessageId;
use cicada_e2e::cluster::{address_text, cluster, eventually, eventually_every, run_id};
use cicada_e2e::docker::compose;
use cicada_e2e::{Class, step};
use cicada_sample_service::e2e::{Recorder, RecorderMsg, RecorderState, Slow, SlowMsg, SlowState};

use crate::{millis, secs};

pub fn class() -> Class {
    Class {
        after_each: Some(step!(restore).1),
        ..Class::new(
            "NodeFaultTest",
            vec![
                step!(kill_9_mid_turn_the_turn_is_re_run_on_another_node_after_the_lease_ttl),
                step!(pause_past_the_lease_another_node_completes_the_stale_commit_is_rejected),
                step!(graceful_stop_a_short_turn_finishes_within_the_grace_period),
                step!(graceful_stop_a_turn_longer_than_the_grace_period_is_released_and_re_run_elsewhere),
            ],
        )
    }
}

pub async fn restore() {
    cluster().restore().await;
}

fn sleep(millis: i64, label: &str) -> SlowMsg {
    SlowMsg::Sleep { millis, label: Some(label.to_owned()) }
}

/// Waits until message `id` runs, and returns that status (with the node holding the lease).
async fn await_running(id: &MessageId) -> MessageStatusView {
    eventually_every(&format!("message {id} running"), secs(30), millis(50), async || {
        let status = cluster().send_only.try_status(id).await?;
        anyhow::ensure!(!status.status.is_finished(), "finished before the fault: {status:?}");
        Ok((status.status == MessageState::Running && status.node.is_some()).then_some(status))
    })
    .await
}

async fn kill_9_mid_turn_the_turn_is_re_run_on_another_node_after_the_lease_ttl() {
    let so = &cluster().send_only;
    let slow = Slow { id: format!("kill-{}", run_id()) };
    so.client.create(&slow, &SlowState::default()).await.unwrap();
    let receipt = so.client.send(&slow, sleep(6_000, "kill")).await.unwrap();
    let running = await_running(&receipt.id).await;
    let victim = running.node.clone().unwrap();
    assert_eq!(running.message.attempt, 1);
    compose::kill(&victim).await.unwrap();
    let killed = Instant::now();

    let last = so.await_finished_within(&receipt.id, secs(60)).await;
    println!("kill: {victim} killed; done after {:.1?}", killed.elapsed());
    assert_eq!(last.status, MessageState::Done);
    assert_eq!(last.message.attempt, 2, "claimed a second time");

    assert_eq!(so.view(&slow).await.version, 1, "exactly one commit");
    let runs = so.state::<SlowState, _>(&slow).await.runs;
    assert_eq!(runs.len(), 1, "{runs:?}");
    let run = &runs[0];
    assert_eq!(run.message_id, receipt.id.as_str());
    assert_eq!(run.attempt, 2);
    assert_ne!(run.node, victim);
    let lease_until = running.lease_until.unwrap();
    assert!(
        run.claimed_at >= lease_until,
        "re-claimed ({}) before the victim's lease ended ({lease_until})",
        run.claimed_at
    );
    assert!(so.failures(&address_text(&slow)).await.is_empty());
}

async fn pause_past_the_lease_another_node_completes_the_stale_commit_is_rejected() {
    let c = cluster();
    let so = &c.send_only;
    let recorder = Recorder { id: format!("pause-{}", run_id()) };
    let recorder_address = address_text(&recorder);
    so.client.create(&recorder, &RecorderState::default()).await.unwrap();
    let message = RecorderMsg::Record { label: "pause".to_owned(), sleep_millis: 4_000 };
    let receipt = so.client.send(&recorder, message).await.unwrap();
    let victim_id = await_running(&receipt.id).await.node.unwrap();
    let victim = c.node(&victim_id);
    // The external side effect happens before the sleep: wait until the victim has done it, then freeze it.
    eventually(&format!("side effect on {victim}"), secs(30), async || {
        Ok((victim.effects(&recorder_address).await?.len() == 1).then_some(()))
    })
    .await;
    compose::pause(&victim_id).await.unwrap();
    // Wait until another node took over (lease expired, claimed again), so unpausing cannot win the race. When this
    // fails, the after-each hook unpauses.
    eventually("taken over by another node", secs(40), async || {
        let status = so.try_status(&receipt.id).await?;
        let taken = status.status.is_finished() || status.node.as_ref().is_some_and(|n| *n != victim_id);
        Ok(taken.then_some(()))
    })
    .await;
    compose::unpause(&victim_id).await;
    let last = so.await_finished_within(&receipt.id, secs(30)).await;
    assert_eq!(last.status, MessageState::Done);
    assert_eq!(last.message.attempt, 2);

    // The unpaused node finishes its sleep (or notices the lost lease) and tries to commit: fenced by the epoch. Its
    // sleep has at most 4 s left; nothing must change in that time.
    eventually(&format!("{victim} healthy again"), secs(30), async || Ok(victim.healthy().await.then_some(()))).await;
    tokio::time::sleep(secs(4)).await;
    let records = so.state::<RecorderState, _>(&recorder).await.records;
    assert_eq!(records.len(), 1, "exactly one committed record: {records:?}");
    assert_eq!(records[0].attempt, 2);
    assert_ne!(records[0].node, victim_id);
    assert_eq!(so.view(&recorder).await.version, 1);

    // External side effects are at-least-once (G4): both nodes performed it.
    let victim_effects = victim.effects(&recorder_address).await.unwrap();
    let winner_effects = c.node(&records[0].node).effects(&recorder_address).await.unwrap();
    assert_eq!(victim_effects.iter().map(|e| e.attempt).collect::<Vec<_>>(), [1]);
    assert_eq!(winner_effects.iter().map(|e| e.attempt).collect::<Vec<_>>(), [2]);
    let ids: HashSet<&str> = victim_effects.iter().chain(&winner_effects).map(|e| e.message_id.as_str()).collect();
    assert_eq!(ids, HashSet::from([receipt.id.as_str()]));
}

async fn graceful_stop_a_short_turn_finishes_within_the_grace_period() {
    let so = &cluster().send_only;
    let slow = Slow { id: format!("stop-short-{}", run_id()) };
    so.client.create(&slow, &SlowState::default()).await.unwrap();
    let receipt = so.client.send(&slow, sleep(2_500, "short")).await.unwrap();
    let victim = await_running(&receipt.id).await.node.unwrap();
    compose::stop(&victim).await.unwrap(); // returns when the container has exited

    let last = so.await_finished_within(&receipt.id, secs(10)).await;
    assert_eq!(last.status, MessageState::Done);
    assert_eq!(last.message.attempt, 1);
    assert_eq!(last.message.lost_claims, 0, "a node that stops in good order loses no claim");
    let runs = so.state::<SlowState, _>(&slow).await.runs;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].node, victim, "committed by the stopping node itself");
    assert_eq!(runs[0].attempt, 1);
}

async fn graceful_stop_a_turn_longer_than_the_grace_period_is_released_and_re_run_elsewhere() {
    let so = &cluster().send_only;
    let slow = Slow { id: format!("stop-long-{}", run_id()) };
    so.client.create(&slow, &SlowState::default()).await.unwrap();
    let receipt = so.client.send(&slow, sleep(9_000, "long")).await.unwrap();
    let victim = await_running(&receipt.id).await.node.unwrap();
    let stopping = Instant::now();
    compose::stop(&victim).await.unwrap();
    println!("stop: {victim} exited after {:.1?}", stopping.elapsed());

    let last = so.await_finished_within(&receipt.id, secs(60)).await;
    assert_eq!(last.status, MessageState::Done);
    assert_eq!(last.message.attempt, 2);
    assert_eq!(last.message.lost_claims, 0, "the stopping node gave its claim back: it is not lost");
    let runs = so.state::<SlowState, _>(&slow).await.runs;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_ne!(runs[0].node, victim);
    assert_eq!(runs[0].attempt, 2);
    assert_eq!(so.view(&slow).await.version, 1);
    assert!(so.failures(&address_text(&slow)).await.is_empty());
}
