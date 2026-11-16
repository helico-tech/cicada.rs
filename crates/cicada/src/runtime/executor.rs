//! Runs claimed turns: load, run the handler with buffered effects under a heartbeat and a timeout, then commit fenced
//! by the claim's epoch.
//!
//! **Lease deadline.** A turn never runs past the lease it holds. Besides the store's own fencing (a commit with a
//! stale epoch is rejected), every turn keeps a *local* deadline on the node's monotonic clock: the time the claim (or
//! the last successful renewal) was *sent*, plus the TTL, minus a safety margin. When the deadline passes without a
//! successful renewal, the handler is cancelled and nothing is committed: by then another node may have claimed the
//! instance (G1). The store reporting the lease lost cancels it at once.
//!
//! **Commit.** A commit that fails with an unavailable store is retried with back-off while the heartbeat keeps the
//! lease, bounded by the lease deadline and by one TTL in total; the handler is never run again for it (G6). Retrying is
//! safe because a commit is fenced and idempotent: if an attempt that failed had in fact applied, the lease is released
//! and the retry returns `LeaseLost`. A commit that cannot get through in time is abandoned *without* releasing the
//! lease, so the message is only handed to another node when the lease expires. A commit the store refuses as invalid
//! data is treated like a rejected commit: the message is committed `failed` without effects.
//!
//! **Retry.** When the handler fails or exceeds its maximum turn, the message's own retry policy decides: with a
//! policy, attempts left and a failure that is not permanent, the commit is a reschedule after the backoff, else
//! `failed`.
//!
//! **Observers.** The node's observers are told that a turn starts once it is claimed, before anything is decoded, and
//! the values they return are told the outcome once its commit succeeded (the `failed` that replaced a rejected or
//! refused commit included); without such a commit they are dropped (see [`crate::observer`]).
//!
//! **Claim limit.** A claimed message with as many lost claims as the node's limit is not decoded or run (ADR 0025): the
//! turn commits it `failed` with the error `claim limit: <n> lost claims`, fenced by its epoch, without asking its retry
//! policy, and only once that commit succeeded are the observers told its start and its end.
//!
//! **A claim that arrives too late.** A claim whose answer comes after its local lease deadline starts nothing (no
//! observer, no load, no handler) and is released: left to run out, it would be a lost claim of its message, and a
//! slow store is not a fault of the message. A claim that arrives with less than one heartbeat of its lease left runs,
//! and the keeper renews it at once instead of one heartbeat later, after its deadline.
//!
//! **Cancel.** An operator's cancel request ([`crate::store::Backend::request_cancel`]) reaches the keeper through its next
//! renewal. If the handler has not ended, the keeper aborts *work* and the turn commits `cancelled`, without effects
//! and fenced by its epoch; the lease is kept throughout, so no other turn can start (G1). If the handler already
//! ended, the request is ignored and the turn commits normally (G6). One atomic [`HandlerPhase`] decides which of the
//! two came first.
//!
//! **After the handler.** Once the handler has returned or failed (or was stopped by its turn timeout), the lease is
//! never released for a re-run (G6): every later failure ends in a `failed` outcome or, when the outcome cannot be
//! committed, in abandoning the turn without release.
//!
//! **Task structure.** Each turn is one task that tells the observers it starts and spawns three children: *work*
//! (load, the handler under its timeout, checks), *keeper* (heartbeat and local-deadline watchdog; it aborts *work*
//! when the lease is lost) and *overtime* (a warning about handlers that ignore cancellation). The turn task awaits
//! *work*, then commits while the keeper keeps renewing.
//!
//! **Holders.** The same executor runs the turns of a node, whose leases live in its store, and of a
//! [`Worker`](super::Worker), whose leases come from a [`TurnSource`] (ADR 0039). The work task ends in a
//! [`TurnReport`]: a node turns it into the decision it commits itself, a worker completes it through its source. A
//! worker's turns follow the TTL each grant names: its heartbeat is a third of the latest confirmed TTL, an extension is
//! accepted only while the confirmed deadline has not passed (so a late answer cannot revive an expired grant), and
//! completion starts only before that deadline, under one fixed budget of the latest confirmed TTL.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use tokio::sync::Notify;
use tokio::task::AbortHandle;
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;

use super::completion::{
    self, CompleteTurnResult, Decision, Refusal, StateReport, TurnFailure, TurnReport, fallback, prepare,
    report_decision, run_report,
};
use super::effects::EffectBuffer;
use super::resolver::TypeResolver;
use super::results::RenewTurnResult;
use super::source::{SourceError, TurnSource};
use crate::actor::{HostedActor, RunEnd, RunResult, panic_message};
use crate::address::{ActorAddress, TypeName};
use crate::observer::{Observed, Observers};
use crate::store::{
    Backend, Claim, CommitRequest, CommitResult, Message, NodeId, RenewResult, Store, StoreError, TurnOutcome,
};
use crate::turn::{HandlerFailure, TurnInfo};
use crate::watch::FailureCause;

const RENEW_RETRY: Duration = Duration::from_secs(1);
const STUCK_AFTER: Duration = Duration::from_secs(5);
const COMMIT_RETRY_INITIAL: Duration = Duration::from_millis(50);
const COMMIT_RETRY_MAX: Duration = Duration::from_secs(2);
const NANO: Duration = Duration::from_nanos(1);
/// The shortest TTL a worker accepts from its source: below it the safe heartbeat would be the whole lease.
pub(crate) const MIN_GRANT: Duration = Duration::from_nanos(2);

/// A hosted type with everything resolved at node start.
#[derive(Clone)]
pub(crate) struct HostedType {
    pub actor: Arc<dyn HostedActor>,
    pub max_turn: Duration,
}

impl std::fmt::Debug for HostedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostedType").field("actor", &self.actor.name()).field("max_turn", &self.max_turn).finish()
    }
}

/// Lease settings of the turns of one node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LeaseSettings {
    /// The lease TTL requested with every claim and renewal.
    pub ttl: Duration,
    /// Interval between renewals.
    pub heartbeat: Duration,
}

/// Subtracted from a lease of `ttl` for a local deadline: a tenth of it, at most one second (ADR 0037).
pub(crate) fn safety_margin(ttl: Duration) -> Duration {
    (ttl / 10).min(Duration::from_secs(1))
}

/// The conservative local deadline of a lease of `ttl` asked for at `sent`, when the monotonic clock can hold it.
pub(crate) fn lease_deadline(sent: Instant, ttl: Duration) -> Option<Instant> {
    sent.checked_add(ttl).map(|end| end - safety_margin(ttl))
}

/// Where the turns of an executor hold their leases: the store of a node, with the node's lease settings, or the turn
/// source of a worker, with the TTL of each grant.
#[derive(Clone)]
pub(crate) enum Holder {
    Store { store: Store, lease: LeaseSettings },
    Source(Arc<dyn TurnSource>),
}

/// A renewal as an executor sees it: the TTL it was granted for and whether an operator asked to cancel, or lost.
enum Renewal {
    Renewed { ttl: Duration, cancel: bool },
    Lost,
}

impl Holder {
    fn is_source(&self) -> bool {
        matches!(self, Holder::Source(_))
    }

    async fn release(&self, actor: &ActorAddress, epoch: i64) -> Result<(), String> {
        match self {
            Holder::Store { store, .. } => store.release(actor, epoch).await.map_err(|e| e.to_string()),
            Holder::Source(source) => source.release_turn(actor, epoch).await.map_err(|e| e.to_string()),
        }
    }

    async fn renew(&self, actor: &ActorAddress, epoch: i64) -> Result<Renewal, String> {
        match self {
            Holder::Store { store, lease } => match store.renew(actor, epoch, lease.ttl).await {
                Ok(RenewResult::Renewed(_)) => Ok(Renewal::Renewed { ttl: lease.ttl, cancel: false }),
                Ok(RenewResult::CancelRequested(_)) => Ok(Renewal::Renewed { ttl: lease.ttl, cancel: true }),
                Ok(RenewResult::LeaseLost) => Ok(Renewal::Lost),
                Err(e) => Err(e.to_string()),
            },
            Holder::Source(source) => match source.renew_turn(actor, epoch).await {
                Ok(RenewTurnResult::Renewed { ttl, cancel_requested, .. }) => {
                    Ok(Renewal::Renewed { ttl, cancel: cancel_requested })
                }
                Ok(RenewTurnResult::LeaseLost) => Ok(Renewal::Lost),
                Err(e) => Err(e.to_string()),
            },
        }
    }

    /// The deadline of a grant of `ttl` asked for at `sent`; `None` for a source's grant that is shorter than
    /// [`MIN_GRANT`] or that the monotonic clock cannot hold.
    pub(crate) fn grant(&self, sent: Instant, ttl: Duration) -> Option<Instant> {
        if self.is_source() && ttl < MIN_GRANT {
            return None;
        }
        lease_deadline(sent, ttl)
    }

    /// The renewal interval for a lease of `ttl`: the node's heartbeat, or a third of a source's TTL (at least 1 ns).
    pub(crate) fn heartbeat(&self, ttl: Duration) -> Duration {
        match self {
            Holder::Store { lease, .. } => lease.heartbeat,
            Holder::Source(_) => (ttl / 3).max(NANO),
        }
    }
}

/// The lease timing of one turn: the local time its claim or lease request was sent, and the TTL it was granted for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LeaseTiming {
    pub sent: Instant,
    pub ttl: Duration,
}

/// True when `claim`'s message has as many lost claims as `max_lost_claims` allows (ADR 0025).
pub(crate) fn at_claim_limit(max_lost_claims: Option<u32>, claim: &Claim) -> bool {
    max_lost_claims.is_some_and(|max| claim.message.lost_claims >= max)
}

/// The outcome and the commit that fail a message at the claim limit without running it: `failed` with the error
/// `claim limit: <n> lost claims` and cause `ClaimLimit`, fenced by the claim's epoch. Logs why.
pub(crate) fn claim_limit_commit(claim: &Claim) -> (TurnOutcome, CommitRequest) {
    let lost = claim.message.lost_claims;
    tracing::warn!(
        actor = %claim.actor,
        message = %claim.message.id,
        lost_claims = lost,
        "Message {} to {} has {lost} lost claims, as many as the claim limit allows: it fails without running",
        claim.message.id,
        claim.actor
    );
    let outcome = TurnOutcome::Failed(format!("claim limit: {lost} lost claims"));
    let request = CommitRequest::new(claim.actor.clone(), claim.epoch, claim.message.id.clone(), outcome.clone())
        .with_cause(FailureCause::ClaimLimit);
    (outcome, request)
}

/// After a claim-limit commit succeeded: wakes the node for the notifications of the failure, then tells the observers
/// the turn's start and its end.
pub(crate) fn claim_limit_committed(
    observers: &Observers,
    wake: &Notify,
    claim: &Claim,
    outcome: &TurnOutcome,
    sent: &[Message],
) {
    wake.notify_one();
    observers.start(&claim.actor, &claim.message, claim.claimed_at).ended(outcome, sent);
}

enum CommitEnd {
    /// The store answered.
    Answered(CommitResult),
    /// The store refused the data; retrying cannot help.
    Refused(String),
    /// The commit kept failing until the lease deadline (or the lease was lost); nothing more is attempted.
    Abandoned,
}

/// The lease of one turn as this node knows it: its local deadline and the latest confirmed TTL, renewed by heartbeat.
/// A *strict* guard (a worker's) accepts an extension, and lets a completion start, only while its confirmed deadline
/// has not passed: checked under one lock, so that a renewal and the watchdog ready together cannot revive a grant.
struct LeaseGuard {
    confirmed: Mutex<(Instant, Duration)>,
    lost: AtomicBool,
    lost_signal: CancellationToken,
    reason: Mutex<String>,
    strict: bool,
}

impl LeaseGuard {
    fn new(deadline: Instant, ttl: Duration, strict: bool) -> Self {
        Self {
            confirmed: Mutex::new((deadline, ttl)),
            lost: AtomicBool::new(false),
            lost_signal: CancellationToken::new(),
            reason: Mutex::new(String::new()),
            strict,
        }
    }

    fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    fn reason(&self) -> String {
        self.reason.lock().map(|r| r.clone()).unwrap_or_default()
    }

    fn confirmed(&self) -> std::sync::MutexGuard<'_, (Instant, Duration)> {
        self.confirmed.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Time left until the local deadline; zero once it passed.
    fn remaining(&self) -> Duration {
        let deadline = self.confirmed().0;
        deadline.saturating_duration_since(Instant::now())
    }

    /// The TTL of the latest confirmed grant.
    fn ttl(&self) -> Duration {
        self.confirmed().1
    }

    /// Moves the deadline to `deadline` when that is later, and records `ttl` as the latest confirmed TTL. A strict
    /// guard refuses once it is lost or its deadline passed, and then marks itself lost: false.
    fn extend(&self, deadline: Instant, ttl: Duration) -> bool {
        let mut confirmed = self.confirmed();
        if self.strict && (self.is_lost() || Instant::now() >= confirmed.0) {
            drop(confirmed);
            self.lose("the renewal came after the local lease deadline");
            return false;
        }
        if deadline > confirmed.0 {
            confirmed.0 = deadline;
        }
        confirmed.1 = ttl;
        true
    }

    /// True while the lease is held: not lost and, for a strict guard, before its deadline (which marks it lost).
    fn live(&self) -> bool {
        let confirmed = self.confirmed();
        if self.is_lost() {
            return false;
        }
        if self.strict && Instant::now() >= confirmed.0 {
            drop(confirmed);
            self.lose("no renewal confirmed before the local lease deadline");
            return false;
        }
        true
    }

    /// Marks the lease lost; true the first time.
    fn lose(&self, why: &str) -> bool {
        if self.lost.swap(true, Ordering::SeqCst) {
            return false;
        }
        if let Ok(mut reason) = self.reason.lock() {
            *reason = why.to_owned();
        }
        self.lost_signal.cancel();
        true
    }
}

/// Which came first: the end of the handler, or a cancel request. Decided once.
struct HandlerPhase(AtomicU8);

const HANDLER_RUNNING: u8 = 0;
const HANDLER_ENDED: u8 = 1;
const CANCEL_REQUESTED: u8 = 2;

impl HandlerPhase {
    fn new() -> Self {
        Self(AtomicU8::new(HANDLER_RUNNING))
    }

    /// Records that the handler ended; false when a cancel request came first.
    fn end(&self) -> bool {
        self.0.compare_exchange(HANDLER_RUNNING, HANDLER_ENDED, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    /// Records a cancel request; false when the handler has already ended (or the request is already recorded).
    fn cancel(&self) -> bool {
        self.0.compare_exchange(HANDLER_RUNNING, CANCEL_REQUESTED, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    fn handler_ended(&self) -> bool {
        self.0.load(Ordering::SeqCst) == HANDLER_ENDED
    }

    fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst) == CANCEL_REQUESTED
    }
}

/// The types a node hosts, by name.
pub(crate) type HostedTypes = Arc<HashMap<TypeName, HostedType>>;

pub(crate) struct TurnExecutor {
    /// Where the leases live: the node's store, or a worker's source.
    pub holder: Holder,
    pub resolver: Arc<TypeResolver>,
    pub hosted: HostedTypes,
    /// The node's observers, in the order they were added.
    pub observers: Observers,
    /// The claim limit: a message with this many lost claims ends `failed` without running. Always `None` for a
    /// worker: the serving system applies its own.
    pub max_lost_claims: Option<u32>,
    /// How long a cancelled turn keeps renewing while its work stops: `None` for a node, which stops renewing at once;
    /// a worker's cancel timeout, after which a turn whose work has not stopped is abandoned to expire.
    pub renew_while_stopping: Option<Duration>,
    /// Wakes the node's dispatch loop: after a commit that sent messages, wrote notifications or had control effects,
    /// or one that may have applied unacknowledged.
    pub wake: Arc<Notify>,
    /// The claim owner of its turns, which [`Turn::node_id`](crate::Turn::node_id) answers: the node's id, or the
    /// worker's.
    pub owner: NodeId,
}

impl TurnExecutor {
    /// Executes one claimed turn to its end. `timing` holds the local time at which the claim (or lease) request was
    /// sent and the TTL it was granted for. `cancel` is the node or worker stopping after its grace period (or a
    /// dropped `run_due_turns`): the worker is aborted and waited for; then, unless the handler already ended or the
    /// lease was lost, the lease is released, or, after an operator's cancel request that came first, the message
    /// completes `cancelled`.
    pub async fn execute(self: Arc<Self>, claim: Claim, timing: LeaseTiming, cancel: CancellationToken) {
        let Some(hosted) = self.hosted.get(claim.actor.type_name()).cloned() else {
            tracing::warn!("Claimed {}, whose type this node does not host", claim.actor);
            let _ = self.holder.release(&claim.actor, claim.epoch).await;
            return;
        };
        // A claim whose answer came after its local deadline: nothing runs under it, and it is given back rather than
        // left to run out, so that a slow store costs the message no lost claim. A release with an epoch that is no
        // longer current changes nothing. A worker's batch is checked before it gets here: its deadline exists.
        let deadline = self.holder.grant(timing.sent, timing.ttl);
        if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
            match self.holder.release(&claim.actor, claim.epoch).await {
                Ok(()) => tracing::warn!(
                    "Claim of {} for {} arrived after its lease deadline; it is given back",
                    claim.actor,
                    claim.message.id
                ),
                Err(e) => tracing::warn!(
                    "Claim of {} for {} arrived after its lease deadline, and giving it back failed; the lease will run \
                     out, and the next claim counts it as lost: {e}",
                    claim.actor,
                    claim.message.id
                ),
            }
            return;
        }
        let Some(deadline) = deadline else { return };
        if let Holder::Store { store, .. } = &self.holder
            && at_claim_limit(self.max_lost_claims, &claim)
        {
            return self.fail_at_claim_limit(store, &claim, deadline, timing.ttl).await;
        }
        let observed = self.observers.start(&claim.actor, &claim.message, claim.claimed_at);
        let info = Arc::new(TurnInfo {
            self_address: claim.actor.clone(),
            message: claim.message.clone(),
            now: claim.claimed_at,
            send_headers: observed.headers().clone(),
            claim_sent: timing.sent,
            hosted: self.hosted.clone(),
            node_id: self.owner.clone(),
        });
        let guard = Arc::new(LeaseGuard::new(deadline, timing.ttl, self.holder.is_source()));
        let phase = Arc::new(HandlerPhase::new());

        let mut work = tokio::spawn(self.clone().decide_or_fail(
            hosted.clone(),
            claim.clone(),
            info,
            guard.clone(),
            phase.clone(),
        ));
        let keeper =
            tokio::spawn(self.clone().keep_lease(guard.clone(), phase.clone(), work.abort_handle(), claim.clone()));
        let overtime = tokio::spawn(overtime_warning(work.abort_handle(), claim.clone(), hosted.max_turn));

        let report = tokio::select! {
            biased;
            result = &mut work => match result {
                // A cancel request came first: *work* was aborted (or ended without yielding) and its result is
                // dropped; the turn completes `cancelled`, fenced by its epoch.
                _ if phase.cancelled() && !guard.is_lost() => Some(TurnReport::Cancelled),
                // The lease was lost while the work ended (it did not yield in time to be cancelled): by now another
                // node may run the turn, so nothing is committed (as a cancelled worker).
                Ok(_) if guard.is_lost() => None,
                Ok(report) => Some(report),
                Err(e) if e.is_cancelled() => None,
                Err(e) => {
                    let when = if phase.handler_ended() { "after its handler ended" } else { "before its handler ran" };
                    let panic = e.try_into_panic().map(|p| panic_message(&*p)).unwrap_or_default();
                    tracing::error!("Turn of {} for {} panicked {when}: {panic}", claim.actor, claim.message.id);
                    Some(failed(FailureCause::Node, format!("Turn failed {when}: panic: {panic}")))
                }
            },
            _ = cancel.cancelled() => {
                // A node stops renewing at once. A worker keeps its keeper (heartbeat and watchdog) while its work
                // stops, so that a `cancelled` completion stays within the lease it confirmed, but not longer than
                // its cancel timeout: then the turn is abandoned, and its lease expires on its own.
                if self.renew_while_stopping.is_none() {
                    keeper.abort();
                }
                overtime.abort();
                work.abort();
                // Wait for the worker to stop (at its next `.await`; code that does not yield runs on until it
                // returns): only then is it known whether the handler ended, and a release cannot let another node
                // run the turn while this one still does.
                let stopped = match self.renew_while_stopping {
                    Some(keep_for) => {
                        let stopped = tokio::time::timeout(keep_for, &mut work).await.is_ok();
                        if !stopped {
                            keeper.abort();
                            guard.lose("its work did not stop within the cancel timeout; it is abandoned");
                            tracing::warn!(
                                "Turn of {} for {} did not stop within {keep_for:?} of its cancellation; it is no \
                                 longer renewed, and its lease expires on its own",
                                claim.actor, claim.message.id
                            );
                        }
                        stopped
                    }
                    None => false,
                };
                if !stopped {
                    let _ = (&mut work).await;
                }
                if guard.is_lost() {
                } else if phase.cancelled() {
                    // An operator's cancel request came first: honour it instead of handing the turn to another node.
                    self.finish(&claim, TurnReport::Cancelled, &guard, observed).await;
                } else if phase.handler_ended() {
                    tracing::info!(
                        "Turn of {} for {} cancelled after its handler ended; the lease is not released but expires",
                        claim.actor, claim.message.id
                    );
                } else {
                    keeper.abort();
                    let _ = self.holder.release(&claim.actor, claim.epoch).await;
                }
                keeper.abort();
                return;
            }
        };
        match report {
            None => tracing::info!("Turn of {} for {} abandoned: {}", claim.actor, claim.message.id, guard.reason()),
            // The heartbeat keeps running while the commit is retried.
            Some(report) => self.finish(&claim, report, &guard, observed).await,
        }
        keeper.abort();
        overtime.abort();
    }

    /// Ends the turn with `report`: a node commits the decision it derives, a worker completes it through its source.
    async fn finish(&self, claim: &Claim, report: TurnReport, guard: &LeaseGuard, observed: Observed) {
        match &self.holder {
            Holder::Store { store, .. } => {
                let retry = claim.message.retry.as_ref().map(|policy| (policy, claim.message.failures));
                let decision = report_decision(report, &claim.actor, &claim.message.id, retry);
                self.commit(store, claim, decision, guard, observed).await
            }
            Holder::Source(source) => self.complete(source.as_ref(), claim, report, guard, observed).await,
        }
    }

    /// Fails a message that reached the claim limit, without decoding or running it: commits `failed` with the error of
    /// the limit, fenced by the claim's epoch, and then tells the observers the start and the end. When that commit does
    /// not succeed they are told nothing, and the next claim finds one more lost claim.
    async fn fail_at_claim_limit(&self, store: &Store, claim: &Claim, deadline: Instant, ttl: Duration) {
        let (outcome, request) = claim_limit_commit(claim);
        let guard = LeaseGuard::new(deadline, ttl, false);
        match self.commit_with_retry(store, claim, &guard, request).await {
            CommitEnd::Answered(CommitResult::Committed { sent, .. }) => {
                claim_limit_committed(&self.observers, &self.wake, claim, &outcome, &sent);
            }
            CommitEnd::Answered(other) => {
                tracing::info!("Claim-limit commit of {} for {}: {other:?}", claim.actor, claim.message.id)
            }
            CommitEnd::Refused(reason) => {
                tracing::error!("Claim-limit commit of {} for {} refused: {reason}", claim.actor, claim.message.id)
            }
            CommitEnd::Abandoned => {}
        }
    }

    /// The keeper: renews the lease, aborts `work` when it is lost, and aborts it when a renewal brings a cancel request
    /// before the handler ended.
    async fn keep_lease(
        self: Arc<Self>,
        guard: Arc<LeaseGuard>,
        phase: Arc<HandlerPhase>,
        work: AbortHandle,
        claim: Claim,
    ) {
        let lose = |why: &str| {
            if guard.lose(why) {
                work.abort();
            }
        };
        let heartbeat = async {
            let mut heartbeat = self.holder.heartbeat(guard.ttl());
            // A claim that arrived with less than a heartbeat of its lease left is renewed at once: one heartbeat later
            // its deadline would have passed, and the claim would be lost (design decision 15). A worker does the same
            // after every renewal, as the TTLs of its grants may differ.
            let mut wait = if guard.remaining() < heartbeat { Duration::ZERO } else { heartbeat };
            loop {
                sleep(wait).await;
                let sent = Instant::now();
                let renewed = match self.holder.renew(&claim.actor, claim.epoch).await {
                    Ok(Renewal::Renewed { ttl, cancel }) => match self.holder.grant(sent, ttl) {
                        Some(deadline) => Ok((deadline, ttl, cancel)),
                        None => Err(format!("the source granted a TTL of {ttl:?}, which cannot be used")),
                    },
                    Ok(Renewal::Lost) => {
                        lose("the store reports it lost");
                        return;
                    }
                    Err(e) => Err(e),
                };
                match renewed {
                    Ok((deadline, ttl, cancel)) => {
                        if !guard.extend(deadline, ttl) {
                            work.abort();
                            return;
                        }
                        heartbeat = self.holder.heartbeat(ttl);
                        wait = if self.holder.is_source() && guard.remaining() < heartbeat {
                            Duration::ZERO
                        } else {
                            heartbeat
                        };
                        if cancel && phase.cancel() {
                            tracing::info!("Cancelling the turn of {} for {}", claim.actor, claim.message.id);
                            work.abort();
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Lease renewal for {} failed; {:?} left: {e}", claim.actor, guard.remaining());
                        wait = (heartbeat / 4).min(RENEW_RETRY).max(NANO);
                    }
                }
            }
        };
        // Watchdog: independent of the renewal calls, which may hang.
        let watchdog = async {
            loop {
                if guard.is_lost() {
                    return;
                }
                let left = guard.remaining();
                if left.is_zero() {
                    lose("no renewal confirmed before the local lease deadline");
                    return;
                }
                sleep(left).await;
            }
        };
        let kept = AssertUnwindSafe(async {
            tokio::select! {
                biased;
                _ = heartbeat => {}
                _ = watchdog => {}
            }
        })
        .catch_unwind()
        .await;
        if let Err(panic) = kept {
            tracing::error!(
                "Lease keeper of {} for {} failed; treating the lease as lost: {}",
                claim.actor,
                claim.message.id,
                panic_message(&*panic)
            );
            lose("its lease keeper failed");
        }
        if guard.is_lost() {
            sleep(STUCK_AFTER).await;
            if !work.is_finished() {
                tracing::warn!(
                    "Handler of {} for {} is still running {STUCK_AFTER:?} after its lease was lost: it does not yield \
                     (blocking code? use spawn_blocking); its turn stays taken until it returns",
                    claim.actor,
                    claim.message.id
                );
            }
        }
    }

    /// [`Self::decide`], with every panic (not a cancellation of this turn) turned into a failure report, so that it
    /// becomes the message's outcome rather than releasing the lease (which would re-run the handler).
    async fn decide_or_fail(
        self: Arc<Self>,
        hosted: HostedType,
        claim: Claim,
        info: Arc<TurnInfo>,
        guard: Arc<LeaseGuard>,
        phase: Arc<HandlerPhase>,
    ) -> TurnReport {
        match AssertUnwindSafe(self.decide(&hosted, &claim, &info, &guard, &phase)).catch_unwind().await {
            Ok(report) => report,
            Err(panic) => {
                let when = if phase.handler_ended() { "after its handler ended" } else { "before its handler ran" };
                let panic = panic_message(&*panic);
                tracing::error!("Turn of {} for {} failed {when}: {panic}", claim.actor, claim.message.id);
                failed(FailureCause::Node, format!("Turn failed {when}: panic: {panic}"))
            }
        }
    }

    /// Loads and runs the turn: the report of its handler, or of the failure around it.
    async fn decide(
        &self,
        hosted: &HostedType,
        claim: &Claim,
        info: &Arc<TurnInfo>,
        guard: &LeaseGuard,
        phase: &HandlerPhase,
    ) -> TurnReport {
        // Load: state, own ref, message.
        let loaded = match hosted.actor.load(claim) {
            Ok(loaded) => loaded,
            Err(reason) => return failed(FailureCause::State, reason),
        };

        // Run.
        let started = Instant::now();
        let ended = tokio::time::timeout(hosted.max_turn, (loaded.run)(info.clone(), self.resolver.clone())).await;
        // Also after the turn timeout: the outcome is failed(timeout), never a re-run. Only a cancel request that came
        // first outranks that.
        if !phase.end() {
            return TurnReport::Cancelled;
        }
        // A handler that only blocked its thread past max_turn returns its value: judge the duration ourselves.
        let overran = started.elapsed() > hosted.max_turn;
        match ended {
            // Cut off at max_turn: a handler failure, retried under the message's policy.
            Err(_) => {
                let failure = HandlerFailure::Timeout { max_turn: hosted.max_turn };
                handler_failed(info, failure, "timeout".into(), true)
            }
            // It ended, but only after max_turn (blocking code): still a timeout. Re-running is allowed only when the
            // handler itself failed; one that returned or vetoed, or a message that did not decode, ends
            // failed(timeout), not retried.
            Ok(run) if overran => {
                let retryable = matches!(&run.end, RunEnd::Threw(f) if !f.is_permanent());
                let failure = HandlerFailure::Timeout { max_turn: hosted.max_turn };
                handler_failed(info, failure, "timeout".into(), retryable)
            }
            Ok(run) => match run.end {
                RunEnd::Undecodable(reason) => {
                    tracing::info!("{} cannot decode {}, which fails: {reason}", info.self_address, info.message.id);
                    failed(FailureCause::Message, reason)
                }
                RunEnd::Threw(failure) => {
                    let reason = failure.reason();
                    handler_failed(info, failure, reason, true)
                }
                RunEnd::Returned => self.after_return(run, guard).await,
            },
        }
    }

    async fn after_return(&self, run: RunResult, guard: &LeaseGuard) -> TurnReport {
        match self.check_typed_effects(&run.effects, guard).await {
            Ok(Some(reason)) => return failed(FailureCause::Commit, reason),
            Ok(None) => {}
            Err(e) => return failed(FailureCause::Node, format!("Turn failed after its handler ended: {e}")),
        }
        run_report(run)
    }

    /// Checks what the compiler cannot: that every created actor's type is registered, and its initial state against
    /// that type.
    async fn check_typed_effects(
        &self,
        effects: &EffectBuffer,
        guard: &LeaseGuard,
    ) -> Result<Option<String>, StoreError> {
        for create in &effects.creates {
            let what = format!("type of {}", create.address);
            let resolved = self.retry_read(guard, &what, || self.resolver.resolve(&create.address)).await?;
            if resolved.is_none() {
                return Ok(Some(format!("Create of {}: its type is not registered", create.address)));
            }
            if let Some(target) = self.hosted.get(create.address.type_name())
                && let Err(e) = target.actor.check_state(&create.state)
            {
                return Ok(Some(format!("Create of {}: the initial state cannot be decoded: {e}", create.address)));
            }
        }
        Ok(None)
    }

    /// Runs a registry read of the phase after the handler, retrying one that fails with back-off while the lease is
    /// held and for at most one TTL.
    async fn retry_read<T, F, Fut>(&self, guard: &LeaseGuard, what: &str, read: F) -> Result<T, StoreError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, StoreError>>,
    {
        let budget = Instant::now().checked_add(guard.ttl());
        let mut backoff = COMMIT_RETRY_INITIAL;
        loop {
            match read().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let budget_left = budget.map_or(Duration::MAX, |b| b.saturating_duration_since(Instant::now()));
                    let left = guard.remaining().min(budget_left);
                    if guard.is_lost() || left.is_zero() || matches!(e, StoreError::Invalid(_)) {
                        return Err(e);
                    }
                    tracing::warn!("Reading the {what} failed; retrying ({left:?} left): {e}");
                    sleep(backoff.min(left)).await;
                    backoff = (backoff * 2).min(COMMIT_RETRY_MAX);
                }
            }
        }
    }

    /// Commits `decision`, or the `failed` that replaces it when the store rejects or refuses it, and tells `observed`
    /// the outcome once a commit succeeded; otherwise `observed` is dropped.
    async fn commit(&self, store: &Store, claim: &Claim, decision: Decision, guard: &LeaseGuard, observed: Observed) {
        let (request, prepared) = prepare(&claim.actor, claim.epoch, &claim.message.id, decision);
        let TurnFailure { cause, reason, .. } = match self.commit_with_retry(store, claim, guard, request).await {
            CommitEnd::Abandoned => return,
            // A retry commit the store cannot apply (the reschedule's delivery time out of range): the handler's
            // failure stands, with the refusal appended, and so does its cause.
            CommitEnd::Refused(reason) => fallback(&prepared, Refusal::Invalid(&reason)),
            CommitEnd::Answered(CommitResult::Committed { sent, .. }) => {
                completion::log_controls(&claim.actor, &claim.message.id, &prepared.controls);
                self.wake_after(&prepared.outcome, &sent, prepared.notifies);
                observed.ended(&prepared.outcome, &sent);
                return;
            }
            CommitEnd::Answered(CommitResult::LeaseLost) => {
                tracing::info!("Commit of {} for {} rejected: lease lost", claim.actor, claim.message.id);
                return;
            }
            CommitEnd::Answered(CommitResult::Rejected(reason)) => fallback(&prepared, Refusal::Rejected(&reason)),
        };
        let failure = TurnOutcome::Failed(reason);
        // Rejected or refused: nothing was applied and the lease is still held; record the failure without effects.
        let request = CommitRequest::new(claim.actor.clone(), claim.epoch, claim.message.id.clone(), failure.clone())
            .with_cause(cause);
        match self.commit_with_retry(store, claim, guard, request).await {
            CommitEnd::Answered(CommitResult::Committed { sent, .. }) => {
                self.wake_after(&failure, &sent, true);
                observed.ended(&failure, &sent);
            }
            CommitEnd::Answered(other) => {
                tracing::warn!("Failure commit of {} for {}: {other:?}", claim.actor, claim.message.id)
            }
            CommitEnd::Refused(reason) => {
                tracing::error!("Failure commit of {} for {} refused: {reason}", claim.actor, claim.message.id)
            }
            CommitEnd::Abandoned => {}
        }
    }

    /// Commits, retrying an attempt that fails with an unavailable store with back-off while the lease deadline allows
    /// and for at most one TTL. A repeated attempt after one that applied returns `LeaseLost`.
    async fn commit_with_retry(
        &self,
        store: &Store,
        claim: &Claim,
        guard: &LeaseGuard,
        request: CommitRequest,
    ) -> CommitEnd {
        let budget = Instant::now() + guard.ttl();
        let mut backoff = COMMIT_RETRY_INITIAL;
        let mut failures = 0;
        loop {
            match store.commit(request.clone()).await {
                Ok(result) => {
                    if failures > 0 && result == CommitResult::LeaseLost {
                        // An earlier attempt may have applied and sent messages nobody was woken up for.
                        self.wake.notify_one();
                        tracing::info!(
                            "Commit of {} for {}: lease no longer held after {failures} failed attempts; an earlier \
                             attempt may have applied",
                            claim.actor,
                            claim.message.id
                        );
                    }
                    return CommitEnd::Answered(result);
                }
                Err(StoreError::Invalid(reason)) => {
                    tracing::warn!("Commit of {} for {} refused: {reason}", claim.actor, claim.message.id);
                    return CommitEnd::Refused(reason);
                }
                Err(e) => {
                    failures += 1;
                    let left = guard.remaining().min(budget.saturating_duration_since(Instant::now()));
                    if guard.is_lost() || left.is_zero() {
                        tracing::warn!(
                            "Commit of {} for {} abandoned after {failures} attempts; the lease will expire: {e}",
                            claim.actor,
                            claim.message.id
                        );
                        return CommitEnd::Abandoned;
                    }
                    tracing::warn!(
                        "Commit of {} for {} failed; retrying ({left:?} left): {e}",
                        claim.actor,
                        claim.message.id
                    );
                    sleep(backoff.min(left)).await;
                    backoff = (backoff * 2).min(COMMIT_RETRY_MAX);
                }
            }
        }
    }

    /// Completes a worker's turn through its source with `report`, its *current report*. One budget of the latest
    /// confirmed TTL is fixed when completion starts, which must be before the confirmed deadline. An unavailable source
    /// is retried with the same report, with back-off, within the budget and the lease; each pending call is raced
    /// against both, and one cut off is uncertain. `Rejected` replaces the report for good with the failure it carries
    /// (a refused retry of a failure included); never the original effects again. Only `Completed` ends `observed`,
    /// with the outcome and sends the source applied. Every other end abandons the turn without releasing its lease.
    async fn complete(
        &self,
        source: &dyn TurnSource,
        claim: &Claim,
        report: TurnReport,
        guard: &LeaseGuard,
        observed: Observed,
    ) {
        let (actor, id) = (&claim.actor, &claim.message.id);
        if !guard.live() {
            tracing::info!("Turn of {actor} for {id} abandoned before its completion: {}", guard.reason());
            return;
        }
        let Some(budget) = Instant::now().checked_add(guard.ttl()) else {
            tracing::warn!("Turn of {actor} for {id} abandoned: its completion budget cannot be represented");
            return;
        };
        let mut report = report;
        let mut backoff = COMMIT_RETRY_INITIAL;
        let mut uncertain = false;
        loop {
            if !guard.live() || Instant::now() >= budget {
                let why = if guard.is_lost() { guard.reason() } else { "its completion budget ran out".to_owned() };
                tracing::warn!("Completion of {actor} for {id} abandoned; the lease will expire: {why}");
                self.wake_if(uncertain);
                return;
            }
            let attempt = source.complete_turn(actor, claim.epoch, id, report.clone());
            let result = tokio::select! {
                biased;
                result = attempt => result,
                _ = guard.lost_signal.cancelled() => {
                    tracing::warn!(
                        "Completion of {actor} for {id} dropped as its lease was lost ({}); it may have applied",
                        guard.reason()
                    );
                    self.wake.notify_one();
                    return;
                }
                _ = tokio::time::sleep_until(budget) => {
                    tracing::warn!(
                        "Completion of {actor} for {id} dropped as its budget ran out; it may have applied, and the lease \
                         will expire"
                    );
                    self.wake.notify_one();
                    return;
                }
            };
            match result {
                Ok(CompleteTurnResult::Completed(completion)) => {
                    let notifies = match &report {
                        TurnReport::Returned { state, effects } if completion.outcome == TurnOutcome::Done => {
                            completion::log_controls(actor, id, &effects.controls);
                            matches!(state, StateReport::Stop) || !effects.controls.is_empty()
                        }
                        _ => matches!(completion.outcome, TurnOutcome::Failed(_)),
                    };
                    self.wake_after(&completion.outcome, &completion.sent, notifies);
                    observed.ended(&completion.outcome, &completion.sent);
                    return;
                }
                Ok(CompleteTurnResult::Rejected(failure)) => {
                    let terminal = matches!(&report, TurnReport::Failed(f) if !f.retryable);
                    if terminal || failure.retryable || completion::check_failure(&failure).is_err() {
                        tracing::error!(
                            "Completion of {actor} for {id}: the source rejected {} with {failure:?}, which breaks the \
                             protocol; the turn is abandoned and the lease will expire",
                            if terminal { "a failure that cannot be replaced" } else { "the report" }
                        );
                        self.wake_if(uncertain);
                        return;
                    }
                    tracing::info!(
                        "Completion of {actor} for {id} rejected; reporting the failure: {}",
                        failure.reason
                    );
                    report = TurnReport::Failed(failure);
                }
                Ok(CompleteTurnResult::LeaseLost) => {
                    if uncertain {
                        tracing::info!(
                            "Completion of {actor} for {id}: lease no longer held after failed attempts; an earlier \
                             attempt may have applied"
                        );
                    } else {
                        tracing::info!("Completion of {actor} for {id} rejected: lease lost");
                    }
                    self.wake_if(uncertain);
                    return;
                }
                Err(SourceError::Unavailable(e)) => {
                    uncertain = true;
                    let left = guard.remaining().min(budget.saturating_duration_since(Instant::now()));
                    if guard.is_lost() || left.is_zero() {
                        continue;
                    }
                    tracing::warn!("Completion of {actor} for {id} failed; retrying ({left:?} left): {e}");
                    sleep(backoff.min(left)).await;
                    backoff = (backoff * 2).min(COMMIT_RETRY_MAX);
                }
                Err(e) => {
                    tracing::warn!("Completion of {actor} for {id} abandoned; the lease will expire: {e}");
                    self.wake_if(uncertain);
                    return;
                }
            }
        }
    }

    /// Wakes the loop after an attempt that may have applied unacknowledged.
    fn wake_if(&self, uncertain: bool) {
        if uncertain {
            self.wake.notify_one();
        }
    }

    /// After a successful commit: wakes the node for the messages sent, for a rescheduled message (which may be due at
    /// once), and for the notifications of a failure, a stop or a delete and the messages a resume makes due
    /// (`notifies`).
    fn wake_after(&self, outcome: &TurnOutcome, sent: &[Message], notifies: bool) {
        if completion::wakes(outcome, sent, notifies) {
            self.wake.notify_one();
        }
    }
}

/// A failure without retry, as the report of a turn.
fn failed(cause: FailureCause, reason: impl Into<String>) -> TurnReport {
    TurnReport::Failed(TurnFailure { cause, retryable: false, reason: reason.into() })
}

/// The handler failed; its effects are discarded. The message's retry policy later decides between a reschedule and
/// `failed`; the failure decides the cause (ADR 0028).
fn handler_failed(info: &TurnInfo, failure: HandlerFailure, reason: String, retryable: bool) -> TurnReport {
    tracing::debug!("Handler of {} failed on {}: {failure}", info.self_address, info.message.id);
    TurnReport::Failed(TurnFailure { cause: failure.cause(), retryable: retryable && !failure.is_permanent(), reason })
}

/// Logs a warning when the handler is still running a while after its maximum turn duration: it does not yield.
async fn overtime_warning(work: AbortHandle, claim: Claim, max_turn: Duration) {
    sleep(max_turn).await;
    sleep(STUCK_AFTER).await;
    if !work.is_finished() {
        tracing::warn!(
            "Handler of {} for {} is still running {STUCK_AFTER:?} after it exceeded its maximum turn duration of \
             {max_turn:?}: it does not yield (blocking code? use spawn_blocking)",
            claim.actor,
            claim.message.id
        );
    }
}
