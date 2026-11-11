//! Checks of G1, G3, G4, G5, G6 and S8 (see `docs/architecture.md`) on the history of a run, G4 also for the
//! notifications of the watcher.
//!
//! The order of the log is the order things happened in: the run is on one thread.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::retry::RetryPolicy;
use crate::store::OutcomeStatus;

use crate::tests::fuzz::faults::{Ending, Event, HandlerEnd};

/// A guarantee that did not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The guarantee.
    pub check: &'static str,
    /// What happened.
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} broken: {}", self.check, self.detail)
    }
}

fn broken<T>(check: &'static str, detail: String) -> Result<T, Violation> {
    Err(Violation { check, detail })
}

/// Runs every check.
pub fn check_all(events: &[Event]) -> Result<(), Violation> {
    g1(events)?;
    g3(events)?;
    g4(events)?;
    g4_notifications(events)?;
    g5(events)?;
    g6(events)?;
    s8(events)
}

/// G1: handlers of one instance do not overlap, a commit applies only with the latest epoch, and once per epoch.
///
/// A removal ends the instance, so a handler still running then does not count against the next instance at that
/// address: the store voids its lease and fences its commit, and it stops at its next heartbeat. Found by the first
/// long run.
pub fn g1(events: &[Event]) -> Result<(), Violation> {
    let mut running: BTreeMap<&str, &str> = BTreeMap::new();
    let mut latest: BTreeMap<&str, i64> = BTreeMap::new();
    let mut committed: BTreeSet<(&str, i64)> = BTreeSet::new();
    for event in events {
        match event {
            Event::HandlerStart { address, id, .. } => {
                if let Some(other) = running.insert(address, id) {
                    return broken("G1", format!("{id} started on {address} while {other} was running"));
                }
            }
            Event::HandlerEnd { address, .. } | Event::Removed { address } => {
                running.remove(address.as_str());
            }
            Event::Claimed { address, epoch, .. } => {
                latest.insert(address, *epoch);
            }
            Event::Committed { address, epoch, .. } => {
                if !committed.insert((address, *epoch)) {
                    return broken("G1", format!("two commits on {address} with epoch {epoch}"));
                }
                if latest.get(address.as_str()) != Some(epoch) {
                    return broken("G1", format!("a commit on {address} with stale epoch {epoch}"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn count<'a>(ids: impl Iterator<Item = &'a str>) -> BTreeMap<&'a str, usize> {
    let mut counts = BTreeMap::new();
    for id in ids {
        *counts.entry(id).or_insert(0) += 1;
    }
    counts
}

/// G3: every accepted message left the mailbox once and has one outcome, and nothing else did.
pub fn g3(events: &[Event]) -> Result<(), Violation> {
    let accepted = count(events.iter().filter_map(|e| match e {
        Event::Accepted { id, .. } => Some(id.as_str()),
        _ => None,
    }));
    let ended = count(events.iter().filter_map(|e| match e {
        Event::Ended { id } => Some(id.as_str()),
        _ => None,
    }));
    let outcomes = count(events.iter().filter_map(|e| match e {
        Event::Outcome { id, .. } => Some(id.as_str()),
        _ => None,
    }));
    for (id, times) in &accepted {
        if *times != 1 {
            return broken("G3", format!("{id} was accepted {times} times"));
        }
        let left = ended.get(id).copied().unwrap_or(0);
        if left != 1 {
            return broken("G3", format!("{id} left the mailbox {left} times"));
        }
        let finished = outcomes.get(id).copied().unwrap_or(0);
        if finished != 1 {
            return broken("G3", format!("{id} has {finished} outcomes"));
        }
    }
    if let Some(id) = ended.keys().chain(outcomes.keys()).find(|id| !accepted.contains_key(*id)) {
        return broken("G3", format!("{id} ended, but was never accepted"));
    }
    Ok(())
}

/// G4: the state of an instance holds the ids of the `done` commits since it was last removed, once each and in order.
pub fn g4(events: &[Event]) -> Result<(), Violation> {
    let mut done: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for event in events {
        match event {
            Event::Committed { address, id, ending: Ending::Done, .. } => {
                done.entry(address).or_default().push(id.clone());
            }
            Event::Removed { address } => {
                done.remove(address.as_str());
            }
            Event::State { address, ids } => {
                let expected = done.get(address.as_str()).map(Vec::as_slice).unwrap_or_default();
                if ids != expected {
                    return broken("G4", format!("{address} holds {ids:?}, its done messages are {expected:?}"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// G4 for watches: every `failed` outcome of a message to a watched actor (a script actor) has exactly one `Failed`
/// for the watcher, pending or finished, whatever commits were repeated or lost, and every `Failed` tells of such an
/// outcome.
pub fn g4_notifications(events: &[Event]) -> Result<(), Violation> {
    let to: BTreeMap<&str, &str> = events
        .iter()
        .filter_map(|e| match e {
            Event::Accepted { id, to } => Some((id.as_str(), to.as_str())),
            _ => None,
        })
        .collect();
    let failed: BTreeSet<&str> = events
        .iter()
        .filter_map(|e| match e {
            Event::Outcome { id, status: OutcomeStatus::Failed }
                if to.get(id.as_str()).is_some_and(|t| t.starts_with("script/")) =>
            {
                Some(id.as_str())
            }
            _ => None,
        })
        .collect();
    let told = count(events.iter().filter_map(|e| match e {
        Event::Notified { kind, of, .. } if kind == "Failed" => Some(of.as_str()),
        _ => None,
    }));
    for id in &failed {
        let times = told.get(id).copied().unwrap_or(0);
        if times != 1 {
            return broken("G4", format!("{id} failed and its watcher has {times} Failed for it"));
        }
    }
    if let Some(of) = told.keys().find(|of| !failed.contains(*of)) {
        return broken("G4", format!("the watcher has a Failed for {of}, which has no failed outcome"));
    }
    Ok(())
}

/// G5: a claim takes a due message that is the head of its mailbox.
pub fn g5(events: &[Event]) -> Result<(), Violation> {
    for event in events {
        if let Event::Claimed { address, id, deliver_at, at, head, .. } = event {
            if at < deliver_at {
                return broken(
                    "G5",
                    format!("{id} on {address} was claimed at {at}, before it was due at {deliver_at}"),
                );
            }
            if head.as_deref() != Some(id) {
                return broken("G5", format!("{id} on {address} was claimed, the head is {head:?}"));
            }
        }
    }
    Ok(())
}

struct ClaimFacts<'a> {
    id: &'a str,
    attempt: u32,
    failures: u32,
    retry: &'a Option<RetryPolicy>,
}

/// G6: a handler runs again only when its retry policy asked for it, or when its turn was lost before a commit. A
/// handler that ended on its own never has its lease released.
pub fn g6(events: &[Event]) -> Result<(), Violation> {
    let mut claims: BTreeMap<(&str, i64), ClaimFacts> = BTreeMap::new();
    let mut ends: BTreeMap<(&str, u32), HandlerEnd> = BTreeMap::new();
    let mut finished: BTreeSet<&str> = BTreeSet::new();
    for event in events {
        match event {
            Event::Claimed { address, epoch, id, attempt, failures, retry, .. } => {
                claims.insert((address, *epoch), ClaimFacts { id, attempt: *attempt, failures: *failures, retry });
            }
            Event::HandlerStart { id, .. } => {
                if finished.contains(id.as_str()) {
                    return broken("G6", format!("{id} ran again after its final commit"));
                }
            }
            Event::HandlerEnd { id, attempt, how, .. } => {
                ends.insert((id, *attempt), *how);
            }
            Event::Released { address, epoch, .. } => {
                let Some(claim) = claims.get(&(address.as_str(), *epoch)) else { continue };
                let end = ends.get(&(claim.id, claim.attempt));
                if end.is_some_and(|how| *how != HandlerEnd::Dropped) {
                    return broken("G6", format!("the lease of {} was released after its handler ended", claim.id));
                }
            }
            Event::Committed { address, epoch, id, ending, .. } => {
                if *ending != Ending::Rescheduled {
                    finished.insert(id);
                    continue;
                }
                let Some(claim) = claims.get(&(address.as_str(), *epoch)) else {
                    return broken("G6", format!("{id} was rescheduled without a claim"));
                };
                match ends.get(&(claim.id, claim.attempt)) {
                    None => return broken("G6", format!("{id} was rescheduled without a handler run")),
                    Some(HandlerEnd::Returned) => {
                        return broken("G6", format!("{id} was rescheduled after its handler returned"));
                    }
                    Some(_) => {}
                }
                let allowed = claim.retry.as_ref().is_some_and(|policy| policy.retries_after(claim.failures + 1));
                if !allowed {
                    return broken("G6", format!("{id} was rescheduled, its retry policy does not ask for that"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// S8: after the drain, no due message is left in a mailbox.
pub fn s8(events: &[Event]) -> Result<(), Violation> {
    for event in events {
        if let Event::StillPending { id, due: true } = event {
            return broken("S8", format!("{id} is due and still pending after the drain"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::retry::Backoff;
    use crate::testing::default_start;

    use super::*;

    const A: &str = "script/a";

    fn s(text: &str) -> String {
        text.to_owned()
    }

    fn claimed(epoch: i64, id: &str, attempt: u32) -> Event {
        Event::Claimed {
            node: s("n0"),
            address: s(A),
            epoch,
            id: s(id),
            attempt,
            failures: 0,
            retry: None,
            deliver_at: default_start(),
            at: default_start(),
            head: Some(s(id)),
        }
    }

    fn retried(event: Event, failures_so_far: u32, max_attempts: u32) -> Event {
        let policy = RetryPolicy::new(max_attempts, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
        match event {
            Event::Claimed { node, address, epoch, id, attempt, deliver_at, at, head, .. } => Event::Claimed {
                node,
                address,
                epoch,
                id,
                attempt,
                failures: failures_so_far,
                retry: Some(policy),
                deliver_at,
                at,
                head,
            },
            other => other,
        }
    }

    fn committed(epoch: i64, id: &str, ending: Ending) -> Event {
        Event::Committed { node: s("n0"), address: s(A), epoch, id: s(id), ending }
    }

    fn start(id: &str, attempt: u32) -> Event {
        Event::HandlerStart { node: s("n0"), address: s(A), id: s(id), attempt }
    }

    fn end(id: &str, attempt: u32, how: HandlerEnd) -> Event {
        Event::HandlerEnd { node: s("n0"), address: s(A), id: s(id), attempt, how }
    }

    fn accepted(id: &str) -> Event {
        Event::Accepted { id: s(id), to: s(A) }
    }

    fn ended(id: &str) -> Event {
        Event::Ended { id: s(id) }
    }

    fn outcome(id: &str) -> Event {
        Event::Outcome { id: s(id), status: OutcomeStatus::Done }
    }

    fn state(ids: &[&str]) -> Event {
        Event::State { address: s(A), ids: ids.iter().map(|i| s(i)).collect() }
    }

    fn released(epoch: i64) -> Event {
        Event::Released { node: s("n0"), address: s(A), epoch }
    }

    /// A run of one message that went well.
    fn good_run() -> Vec<Event> {
        vec![
            accepted("m"),
            claimed(1, "m", 1),
            start("m", 1),
            end("m", 1, HandlerEnd::Returned),
            committed(1, "m", Ending::Done),
            ended("m"),
            outcome("m"),
            state(&["m"]),
        ]
    }

    fn failure(result: Result<(), Violation>, check: &str, part: &str) {
        let violation = result.expect_err("the log is bad");
        assert_eq!(violation.check, check);
        assert!(violation.detail.contains(part), "{violation}");
    }

    #[test]
    fn a_good_run_passes_every_check() {
        check_all(&good_run()).unwrap();
    }

    #[test]
    fn g1_refuses_overlapping_handlers() {
        let log = [start("m", 1), start("n", 1)];
        failure(g1(&log), "G1", "while m was running");
    }

    #[test]
    fn g1_accepts_handlers_one_after_the_other() {
        g1(&[start("m", 1), end("m", 1, HandlerEnd::Dropped), start("m", 2)]).unwrap();
    }

    #[test]
    fn g1_counts_handlers_per_instance_and_a_removal_ends_the_instance() {
        g1(&[start("m", 1), Event::Removed { address: s(A) }, start("n", 1)]).unwrap();
    }

    #[test]
    fn g1_refuses_two_commits_in_one_epoch() {
        let log = [claimed(1, "m", 1), committed(1, "m", Ending::Rescheduled), committed(1, "m", Ending::Done)];
        failure(g1(&log), "G1", "two commits");
    }

    #[test]
    fn g1_refuses_a_stale_commit() {
        let log = [claimed(1, "m", 1), claimed(2, "m", 2), committed(1, "m", Ending::Done)];
        failure(g1(&log), "G1", "stale epoch");
    }

    #[test]
    fn g3_refuses_a_message_with_no_outcome() {
        failure(g3(&[accepted("m"), ended("m")]), "G3", "0 outcomes");
    }

    #[test]
    fn g3_refuses_a_message_that_never_ended() {
        failure(g3(&[accepted("m"), outcome("m")]), "G3", "left the mailbox 0 times");
    }

    #[test]
    fn g3_refuses_a_message_that_ended_twice() {
        failure(g3(&[accepted("m"), ended("m"), ended("m"), outcome("m")]), "G3", "left the mailbox 2 times");
    }

    #[test]
    fn g3_refuses_two_outcomes() {
        failure(g3(&[accepted("m"), ended("m"), outcome("m"), outcome("m")]), "G3", "2 outcomes");
    }

    #[test]
    fn g3_refuses_a_message_accepted_twice() {
        failure(g3(&[accepted("m"), accepted("m"), ended("m"), outcome("m")]), "G3", "accepted 2 times");
    }

    #[test]
    fn g3_refuses_an_outcome_of_a_message_never_accepted() {
        failure(g3(&[ended("m"), outcome("m")]), "G3", "never accepted");
    }

    #[test]
    fn g4_refuses_an_effect_applied_twice() {
        failure(g4(&[committed(1, "m", Ending::Done), state(&["m", "m"])]), "G4", "holds");
    }

    #[test]
    fn g4_refuses_an_effect_of_a_message_not_done() {
        failure(g4(&[committed(1, "m", Ending::Failed), state(&["m"])]), "G4", "holds");
    }

    #[test]
    fn g4_refuses_a_done_message_without_its_effect() {
        failure(g4(&[committed(1, "m", Ending::Done), state(&[])]), "G4", "holds");
    }

    #[test]
    fn g4_starts_over_after_a_removal() {
        g4(&[committed(1, "m", Ending::Done), Event::Removed { address: s(A) }, state(&[])]).unwrap();
    }

    #[test]
    fn g5_refuses_a_claim_before_the_delivery_time() {
        let early = match claimed(1, "m", 1) {
            Event::Claimed { node, address, epoch, id, attempt, failures, retry, deliver_at, head, .. } => {
                Event::Claimed {
                    node,
                    address,
                    epoch,
                    id,
                    attempt,
                    failures,
                    retry,
                    deliver_at: deliver_at + chrono::Duration::seconds(1),
                    at: deliver_at,
                    head,
                }
            }
            other => other,
        };
        failure(g5(&[early]), "G5", "before it was due");
    }

    #[test]
    fn g5_refuses_a_claim_that_is_not_the_head() {
        let log = [match claimed(1, "m", 1) {
            Event::Claimed { node, address, epoch, id, attempt, failures, retry, deliver_at, at, .. } => {
                Event::Claimed {
                    node,
                    address,
                    epoch,
                    id,
                    attempt,
                    failures,
                    retry,
                    deliver_at,
                    at,
                    head: Some(s("older")),
                }
            }
            other => other,
        }];
        failure(g5(&log), "G5", "the head is");
    }

    #[test]
    fn g6_refuses_a_run_after_the_final_commit() {
        let log = [claimed(1, "m", 1), committed(1, "m", Ending::Done), start("m", 2)];
        failure(g6(&log), "G6", "after its final commit");
    }

    #[test]
    fn g6_refuses_a_retry_after_the_handler_returned() {
        let log = [
            retried(claimed(1, "m", 1), 0, 3),
            start("m", 1),
            end("m", 1, HandlerEnd::Returned),
            committed(1, "m", Ending::Rescheduled),
        ];
        failure(g6(&log), "G6", "after its handler returned");
    }

    #[test]
    fn g6_refuses_a_retry_without_a_policy() {
        let log = [
            claimed(1, "m", 1),
            start("m", 1),
            end("m", 1, HandlerEnd::Failed),
            committed(1, "m", Ending::Rescheduled),
        ];
        failure(g6(&log), "G6", "does not ask for that");
    }

    #[test]
    fn g6_refuses_a_retry_past_the_policy() {
        let log = [
            retried(claimed(3, "m", 3), 2, 3),
            start("m", 3),
            end("m", 3, HandlerEnd::Failed),
            committed(3, "m", Ending::Rescheduled),
        ];
        failure(g6(&log), "G6", "does not ask for that");
    }

    #[test]
    fn g6_accepts_a_retry_the_policy_asks_for() {
        g6(&[
            retried(claimed(1, "m", 1), 0, 3),
            start("m", 1),
            end("m", 1, HandlerEnd::Panicked),
            committed(1, "m", Ending::Rescheduled),
            retried(claimed(2, "m", 2), 1, 3),
            start("m", 2),
            end("m", 2, HandlerEnd::Returned),
            committed(2, "m", Ending::Done),
        ])
        .unwrap();
    }

    #[test]
    fn g6_accepts_a_run_after_a_turn_lost_before_its_commit() {
        g6(&[
            claimed(1, "m", 1),
            start("m", 1),
            end("m", 1, HandlerEnd::Returned),
            claimed(2, "m", 2),
            start("m", 2),
            end("m", 2, HandlerEnd::Returned),
            committed(2, "m", Ending::Done),
        ])
        .unwrap();
    }

    #[test]
    fn g6_refuses_releasing_the_lease_after_the_handler_ended() {
        let log = [claimed(1, "m", 1), start("m", 1), end("m", 1, HandlerEnd::Returned), released(1)];
        failure(g6(&log), "G6", "was released after its handler ended");
    }

    #[test]
    fn g6_accepts_releasing_the_lease_of_a_handler_that_was_stopped() {
        g6(&[claimed(1, "m", 1), start("m", 1), end("m", 1, HandlerEnd::Dropped), released(1)]).unwrap();
    }

    fn failed_outcome(id: &str) -> Event {
        Event::Outcome { id: s(id), status: OutcomeStatus::Failed }
    }

    fn notified(id: &str, of: &str) -> Event {
        Event::Notified { id: s(id), kind: s("Failed"), of: s(of) }
    }

    #[test]
    fn g4_accepts_one_failed_per_failed_message_of_a_watched_actor() {
        let log = [
            accepted("m"),
            Event::Accepted { id: s("e.0"), to: s("watcher/_") },
            failed_outcome("m"),
            notified("e.0", "m"),
        ];
        g4_notifications(&log).unwrap();
        let unwatched = [Event::Accepted { id: s("n"), to: s("watcher/_") }, failed_outcome("n")];
        g4_notifications(&unwatched).unwrap();
    }

    #[test]
    fn g4_refuses_a_failure_the_watcher_was_not_told_of_or_told_twice() {
        failure(g4_notifications(&[accepted("m"), failed_outcome("m")]), "G4", "has 0 Failed");
        let twice = [accepted("m"), failed_outcome("m"), notified("e.0", "m"), notified("f.0", "m")];
        failure(g4_notifications(&twice), "G4", "has 2 Failed");
    }

    #[test]
    fn g4_refuses_a_failed_for_a_message_that_did_not_fail() {
        failure(g4_notifications(&[accepted("m"), outcome("m"), notified("e.0", "m")]), "G4", "no failed outcome");
    }

    #[test]
    fn s8_refuses_a_due_message_after_the_drain() {
        failure(s8(&[Event::StillPending { id: s("m"), due: true }]), "S8", "still pending");
    }

    #[test]
    fn s8_accepts_a_message_that_is_not_due() {
        s8(&[Event::StillPending { id: s("m"), due: false }]).unwrap();
    }
}
