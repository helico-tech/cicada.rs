//! Sending `Step`s to counters with fail-over, and the G1/G3/G5 checks on a counter's record.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use cicada::SendOptions;
use cicada::api::MessageStatusView;
use cicada::store::MessageId;
use cicada_sample_service::e2e::{Counter, CounterMsg, CounterState};

use crate::cluster::Node;

/// One `Step` that was accepted, with the payload `seq` the counter records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentStep {
    /// The target.
    pub counter: Counter,
    /// The step's seq.
    pub seq: i64,
    /// The message id.
    pub id: MessageId,
}

/// Sends `Step { seq, hold_millis }` to `counter` through `nodes`, starting with the first; a node that cannot be
/// reached (killed, stopped) is skipped and the *same id* is sent through the next one, so a send whose
/// acknowledgement was lost is deduplicated by the store (200) instead of enqueued twice. Three rounds, 0.5 s, then
/// 1 s apart.
///
/// # Panics
///
/// When no node accepted the step.
pub async fn send_step(nodes: &[&Node], counter: &Counter, seq: i64, hold_millis: i64, delay: Duration) -> SentStep {
    let id = MessageId::random();
    let mut last_error = None;
    for round in 1..=3u64 {
        for node in nodes {
            let options = SendOptions::new().id(id.clone());
            let options = if delay.is_zero() { options } else { options.delay(delay) };
            match node.client.send_with(counter, CounterMsg::Step { seq, hold_millis }, options).await {
                Ok(receipt) => {
                    assert_eq!(receipt.id, id);
                    return SentStep { counter: counter.clone(), seq, id };
                }
                Err(e) => last_error = Some(format!("{node}: {e}")),
            }
        }
        tokio::time::sleep(Duration::from_millis(500 * round)).await;
    }
    let nodes: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    panic!("Step {seq} to {} could not be sent through any of {nodes:?}: {last_error:?}", counter.id)
}

/// Checks G1 and G5 for one counter: every accepted step was handled exactly once (G3), no two turns overlapped (G1),
/// nothing was claimed before it was due, and the handling order is the mailbox order the store assigned, i.e. sorted
/// by `(deliverAt, seq)` (G5).
///
/// Why the global sort is the right expectation even for messages that arrive while others are handled: a message
/// enqueued after a turn was claimed has `deliverAt >= store now >` that turn's `deliverAt` (clamped to now), and a
/// later `seq` on equal times, so it sorts after every message handled before it.
///
/// # Panics
///
/// When a check fails.
pub fn assert_mailbox_order(
    counter: &Counter,
    sent: &[SentStep],
    statuses: &HashMap<MessageId, MessageStatusView>,
    state: &CounterState,
) {
    let name = &counter.id;
    let steps: Vec<&SentStep> = sent.iter().filter(|s| &s.counter == counter).collect();
    assert_eq!(steps.len() as i64, state.count, "{name}: every accepted step committed exactly once");
    assert_eq!(state.overlaps, 0, "{name}: overlapping turns");
    let handled: Vec<i64> = state.entries.iter().map(|e| e.seq).collect();
    let distinct: HashSet<i64> = handled.iter().copied().collect();
    assert_eq!(handled.len(), distinct.len(), "{name}: a step was handled twice: {handled:?}");
    let accepted: HashSet<i64> = steps.iter().map(|s| s.seq).collect();
    assert_eq!(accepted, distinct, "{name}: handled steps differ from the accepted ones");

    let envelope: HashMap<i64, &cicada::api::MessageView> =
        steps.iter().map(|s| (s.seq, &statuses.get(&s.id).expect("a status per step").message)).collect();
    let mut expected: Vec<i64> = steps.iter().map(|s| s.seq).collect();
    expected.sort_by_key(|seq| (envelope[seq].deliver_at, envelope[seq].seq));
    assert_eq!(handled, expected, "{name}: handling order differs from (deliverAt, seq)");
    for entry in &state.entries {
        let due = envelope[&entry.seq].deliver_at;
        assert!(entry.claimed_at >= due, "{name}: step {} claimed at {}, due {due}", entry.seq, entry.claimed_at);
    }
    // A turn starts after the previous one of the same instance committed (store time vs node clocks of one host).
    for pair in state.entries.windows(2) {
        assert!(pair[1].claimed_at >= pair[0].claimed_at, "{name}: claims not monotonic: {:?}, {:?}", pair[0], pair[1]);
    }
}
