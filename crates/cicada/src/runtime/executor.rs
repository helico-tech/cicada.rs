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
//! `failed`. The `on_handler_error` hooks then see that decision.
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
//! **Task structure.** Each turn is one task that spawns three children: *work* (load, `on_claim`, the handler under
//! its timeout, checks), *keeper* (heartbeat and local-deadline watchdog; it aborts *work* when the lease is
//! lost) and *overtime* (a warning about handlers that ignore cancellation). The turn task awaits *work*, then commits
//! while the keeper keeps renewing.

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

use super::effects::EffectBuffer;
use super::limits;
use super::resolver::TypeResolver;
use crate::actor::{HostedActor, RunEnd, RunResult, panic_message};
use crate::address::TypeName;
use crate::message;
use crate::plugin::{Attributes, ClaimContext, HandlerErrorContext, HookResult, OutcomeContext, Plugin, TurnInfo};
use crate::store::{
    ActorCreate, Backend, Claim, CommitRequest, CommitResult, Delivery, Message, MessageId, OutgoingMessage,
    RenewResult, StateChange, Store, StoreError, TurnOutcome,
};
use crate::turn::HandlerFailure;

const RENEW_RETRY: Duration = Duration::from_secs(1);
const STUCK_AFTER: Duration = Duration::from_secs(5);
const COMMIT_RETRY_INITIAL: Duration = Duration::from_millis(50);
const COMMIT_RETRY_MAX: Duration = Duration::from_secs(2);

/// A hosted type with everything resolved at node start.
#[derive(Clone)]
pub(crate) struct HostedType {
    pub actor: Arc<dyn HostedActor>,
    pub max_turn: Duration,
}

/// Lease settings of the turns of one node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LeaseSettings {
    /// The lease TTL requested with every claim and renewal.
    pub ttl: Duration,
    /// Interval between renewals.
    pub heartbeat: Duration,
}

impl LeaseSettings {
    /// Subtracted from every local lease deadline: covers clock-rate differences and the store's processing time.
    fn safety_margin(&self) -> Duration {
        (self.ttl / 10).min(Duration::from_secs(1))
    }
}

/// What a turn decided, before it is committed.
struct Decision {
    outcome: TurnOutcome,
    state: StateChange,
    creates: Vec<ActorCreate>,
    sends: Vec<OutgoingMessage>,
    cancels: Vec<MessageId>,
    /// The handler's failure reason when the turn failed: the fallback if the retry's reschedule is refused.
    handler_failure: Option<String>,
}

impl Decision {
    fn failed(reason: impl Into<String>) -> Self {
        Self::of(TurnOutcome::Failed(reason.into()))
    }

    fn of(outcome: TurnOutcome) -> Self {
        Self {
            outcome,
            state: StateChange::Unchanged,
            creates: Vec::new(),
            sends: Vec::new(),
            cancels: Vec::new(),
            handler_failure: None,
        }
    }
}

enum CommitEnd {
    /// The store answered.
    Answered(CommitResult),
    /// The store refused the data; retrying cannot help.
    Refused(String),
    /// The commit kept failing until the lease deadline (or the lease was lost); nothing more is attempted.
    Abandoned,
}

/// The lease of one turn as this node knows it: its local deadline, renewed by heartbeat.
struct LeaseGuard {
    deadline: Mutex<Instant>,
    lost: AtomicBool,
    reason: Mutex<String>,
}

impl LeaseGuard {
    fn new(deadline: Instant) -> Self {
        Self { deadline: Mutex::new(deadline), lost: AtomicBool::new(false), reason: Mutex::new(String::new()) }
    }

    fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    fn reason(&self) -> String {
        self.reason.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Time left until the local deadline; zero once it passed.
    fn remaining(&self) -> Duration {
        let deadline = *self.deadline.lock().unwrap_or_else(|e| e.into_inner());
        deadline.saturating_duration_since(Instant::now())
    }

    fn extend(&self, deadline: Instant) {
        let mut current = self.deadline.lock().unwrap_or_else(|e| e.into_inner());
        if deadline > *current {
            *current = deadline;
        }
    }

    /// Marks the lease lost; true the first time.
    fn lose(&self, why: &str) -> bool {
        if self.lost.swap(true, Ordering::SeqCst) {
            return false;
        }
        if let Ok(mut reason) = self.reason.lock() {
            *reason = why.to_owned();
        }
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

pub(crate) struct TurnExecutor {
    pub store: Store,
    pub resolver: Arc<TypeResolver>,
    pub hosted: HashMap<TypeName, HostedType>,
    /// The node's plugins, in installation order.
    pub plugins: Vec<Arc<dyn Plugin>>,
    pub lease: LeaseSettings,
    /// Wakes the node's dispatch loop: after a commit that sent messages, or one that may have applied unacknowledged.
    pub wake: Arc<Notify>,
}

impl TurnExecutor {
    /// Executes one claimed turn to its end. `claim_sent` is the local time at which the claim request was sent.
    /// `cancel` is the node stopping after its grace period (or a dropped `run_due_turns`): the worker is aborted and
    /// waited for; then, unless the handler already ended or the lease was lost, the lease is released, or, after an
    /// operator's cancel request that came first, the message commits `cancelled`.
    pub async fn execute(self: Arc<Self>, claim: Claim, claim_sent: Instant, cancel: CancellationToken) {
        let Some(hosted) = self.hosted.get(claim.actor.type_name()).cloned() else {
            tracing::warn!("Claimed {}, whose type this node does not host", claim.actor);
            let _ = self.store.release(&claim.actor, claim.epoch).await;
            return;
        };
        let info = Arc::new(TurnInfo {
            self_address: claim.actor.clone(),
            message: claim.message.clone(),
            now: claim.claimed_at,
            attributes: Attributes::default(),
            send_headers: Mutex::default(),
            unhandled: std::sync::OnceLock::new(),
            claim_sent,
        });
        let guard = Arc::new(LeaseGuard::new(claim_sent + self.lease.ttl - self.lease.safety_margin()));
        let phase = Arc::new(HandlerPhase::new());

        let mut work = tokio::spawn(self.clone().decide_or_fail(
            hosted.clone(),
            claim.clone(),
            info.clone(),
            guard.clone(),
            phase.clone(),
        ));
        let keeper =
            tokio::spawn(self.clone().keep_lease(guard.clone(), phase.clone(), work.abort_handle(), claim.clone()));
        let overtime = tokio::spawn(overtime_warning(work.abort_handle(), claim.clone(), hosted.max_turn));

        let decision = tokio::select! {
            biased;
            result = &mut work => match result {
                // A cancel request came first: *work* was aborted (or ended without yielding) and its result is
                // dropped; the turn commits `cancelled`, fenced by its epoch.
                _ if phase.cancelled() && !guard.is_lost() => Some(Decision::of(TurnOutcome::Cancelled)),
                // The lease was lost while the work ended (it did not yield in time to be cancelled): by now another
                // node may run the turn, so nothing is committed (as a cancelled worker).
                Ok(_) if guard.is_lost() => None,
                Ok(decision) => Some(decision),
                Err(e) if e.is_cancelled() => None,
                Err(e) => {
                    let when = if phase.handler_ended() { "after its handler ended" } else { "before its handler ran" };
                    let panic = e.try_into_panic().map(|p| panic_message(&*p)).unwrap_or_default();
                    tracing::error!("Turn of {} for {} panicked {when}: {panic}", claim.actor, claim.message.id);
                    Some(Decision::failed(format!("Turn failed {when}: panic: {panic}")))
                }
            },
            _ = cancel.cancelled() => {
                keeper.abort();
                overtime.abort();
                work.abort();
                // Wait for the worker to stop (at its next `.await`; code that does not yield runs on until it
                // returns): only then is it known whether the handler ended, and a release cannot let another node
                // run the turn while this one still does.
                let _ = (&mut work).await;
                if guard.is_lost() {
                } else if phase.cancelled() {
                    // An operator's cancel request came first: honour it instead of handing the turn to another node.
                    self.commit(&claim, &info, Decision::of(TurnOutcome::Cancelled), &guard).await;
                } else if phase.handler_ended() {
                    tracing::info!(
                        "Turn of {} for {} cancelled after its handler ended; the lease is not released but expires",
                        claim.actor, claim.message.id
                    );
                } else {
                    let _ = self.store.release(&claim.actor, claim.epoch).await;
                }
                return;
            }
        };
        match decision {
            None => tracing::info!("Turn of {} for {} abandoned: {}", claim.actor, claim.message.id, guard.reason()),
            // The heartbeat keeps running while the commit is retried.
            Some(decision) => self.commit(&claim, &info, decision, &guard).await,
        }
        keeper.abort();
        overtime.abort();
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
            let mut wait = self.lease.heartbeat;
            loop {
                sleep(wait).await;
                let sent = Instant::now();
                match self.store.renew(&claim.actor, claim.epoch, self.lease.ttl).await {
                    Ok(result @ (RenewResult::Renewed(_) | RenewResult::CancelRequested(_))) => {
                        guard.extend(sent + self.lease.ttl - self.lease.safety_margin());
                        wait = self.lease.heartbeat;
                        if matches!(result, RenewResult::CancelRequested(_)) && phase.cancel() {
                            tracing::info!("Cancelling the turn of {} for {}", claim.actor, claim.message.id);
                            work.abort();
                        }
                    }
                    Ok(RenewResult::LeaseLost) => {
                        lose("the store reports it lost");
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("Lease renewal for {} failed; {:?} left: {e}", claim.actor, guard.remaining());
                        wait = (self.lease.heartbeat / 4).min(RENEW_RETRY);
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

    /// [`Self::decide`], with every panic (not a cancellation of this turn) turned into a `failed` decision, so that
    /// it is committed as the message's outcome rather than releasing the lease (which would re-run the handler).
    async fn decide_or_fail(
        self: Arc<Self>,
        hosted: HostedType,
        claim: Claim,
        info: Arc<TurnInfo>,
        guard: Arc<LeaseGuard>,
        phase: Arc<HandlerPhase>,
    ) -> Decision {
        match AssertUnwindSafe(self.decide(&hosted, &claim, &info, &guard, &phase)).catch_unwind().await {
            Ok(decision) => decision,
            Err(panic) => {
                let when = if phase.handler_ended() { "after its handler ended" } else { "before its handler ran" };
                let panic = panic_message(&*panic);
                tracing::error!("Turn of {} for {} failed {when}: {panic}", claim.actor, claim.message.id);
                Decision::failed(format!("Turn failed {when}: panic: {panic}"))
            }
        }
    }

    async fn decide(
        &self,
        hosted: &HostedType,
        claim: &Claim,
        info: &Arc<TurnInfo>,
        guard: &LeaseGuard,
        phase: &HandlerPhase,
    ) -> Decision {
        // Load: state, own ref, message.
        let loaded = match hosted.actor.load(claim) {
            Ok(loaded) => loaded,
            Err(reason) => return Decision::failed(reason),
        };
        if let Some(reason) = &loaded.unhandled {
            let _ = info.unhandled.set(reason.clone());
        }

        for plugin in &self.plugins {
            if let Err(e) = guarded(plugin.on_claim(&ClaimContext { info })).await {
                return Decision::failed(format!("on_claim of plugin {} failed: {e}", plugin.name()));
            }
        }

        // Run.
        let started = Instant::now();
        let ended = tokio::time::timeout(hosted.max_turn, (loaded.run)(info.clone(), self.resolver.clone())).await;
        // Also after the turn timeout: the outcome is failed(timeout), never a re-run. Only a cancel request that came
        // first outranks that.
        if !phase.end() {
            return Decision::of(TurnOutcome::Cancelled);
        }
        // A handler that only blocked its thread past max_turn returns its value: judge the duration ourselves.
        let overran = started.elapsed() > hosted.max_turn;
        match ended {
            // Cut off at max_turn: a handler failure, retried under the message's policy, seen by the error hooks.
            Err(_) => {
                let failure = HandlerFailure::Timeout { max_turn: hosted.max_turn, cause: None };
                self.on_handler_error(info, failure, "timeout".into(), true).await
            }
            // It ended, but only after max_turn (blocking code): still a timeout. Re-running is allowed only when the
            // handler itself failed; one that returned or vetoed, or a message that did not decode, ends failed(timeout),
            // not retried.
            Ok(run) if overran => {
                let cause = match run.end {
                    RunEnd::Threw(f) => Some(Box::new(f)),
                    _ => None,
                };
                let retryable = cause.as_ref().is_some_and(|c| !c.is_permanent());
                let failure = HandlerFailure::Timeout { max_turn: hosted.max_turn, cause };
                self.on_handler_error(info, failure, "timeout".into(), retryable).await
            }
            Ok(run) => match run.end {
                RunEnd::Undecodable(reason) => {
                    tracing::info!("{} cannot decode {}, which fails: {reason}", info.self_address, info.message.id);
                    Decision::failed(reason)
                }
                RunEnd::Threw(failure) => {
                    let reason = failure.reason();
                    self.on_handler_error(info, failure, reason, true).await
                }
                RunEnd::Returned => self.after_return(run, guard).await,
            },
        }
    }

    /// The handler failed; its effects are discarded. The message's retry policy decides between a reschedule and
    /// `failed`; then every `on_handler_error` hook sees that decision. A failing hook is logged and ignored.
    async fn on_handler_error(
        &self,
        info: &TurnInfo,
        failure: HandlerFailure,
        reason: String,
        retryable: bool,
    ) -> Decision {
        tracing::debug!("Handler of {} failed on {}: {failure}", info.self_address, info.message.id);
        let failed_runs = info.message.failures + 1; // including this one
        let retry_delay = match &info.message.retry {
            Some(policy) if retryable && !failure.is_permanent() && policy.retries_after(failed_runs) => {
                Some(policy.backoff().delay(failed_runs).min(crate::time::MAX_DELAY))
            }
            _ => None,
        };
        for plugin in &self.plugins {
            let ctx = HandlerErrorContext { info, error: &failure, retry_delay };
            if let Err(e) = guarded(plugin.on_handler_error(&ctx)).await {
                tracing::warn!("on_handler_error of plugin {} failed; ignored: {e}", plugin.name());
            }
        }
        let outcome = match retry_delay {
            Some(delay) => TurnOutcome::Rescheduled { delivery: Delivery::After(delay), error: reason.clone() },
            None => TurnOutcome::Failed(reason.clone()),
        };
        Decision { handler_failure: Some(reason), ..Decision::of(outcome) }
    }

    async fn after_return(&self, run: RunResult, guard: &LeaseGuard) -> Decision {
        let effects = run.effects;
        match self.check_typed_effects(&effects, guard).await {
            Ok(Some(reason)) => return Decision::failed(reason),
            Ok(None) => {}
            Err(e) => return Decision::failed(format!("Turn failed after its handler ended: {e}")),
        }
        let state = if effects.stop {
            StateChange::Stop
        } else {
            match run.new_state {
                None => StateChange::Unchanged,
                Some(Err(e)) => return Decision::failed(format!("State cannot be encoded: {e}")),
                Some(Ok(state)) => {
                    if let Err(e) = message::check_not_null("state", &state)
                        .and_then(|()| limits::check_json("state", Some(&state)))
                    {
                        return Decision::failed(format!("State cannot be encoded: {e}"));
                    }
                    StateChange::NewState(state)
                }
            }
        };
        Decision {
            outcome: TurnOutcome::Done,
            state,
            creates: effects.creates,
            sends: effects.sends,
            cancels: effects.cancels,
            handler_failure: None,
        }
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

    /// Runs a store read of the phase after the handler, retrying one that fails with back-off while the lease is
    /// held and for at most one TTL.
    async fn retry_read<T, F, Fut>(&self, guard: &LeaseGuard, what: &str, read: F) -> Result<T, StoreError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, StoreError>>,
    {
        let budget = Instant::now() + self.lease.ttl;
        let mut backoff = COMMIT_RETRY_INITIAL;
        loop {
            match read().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let left = guard.remaining().min(budget.saturating_duration_since(Instant::now()));
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

    async fn commit(&self, claim: &Claim, info: &TurnInfo, decision: Decision, guard: &LeaseGuard) {
        // Every backend can store the reason (PostgreSQL refuses NUL characters in text). A rescheduled message carries
        // its `lastError` along, so that one is also bounded.
        let outcome = match decision.outcome {
            TurnOutcome::Failed(e) => TurnOutcome::Failed(sanitize(&e)),
            TurnOutcome::Rescheduled { delivery, error } => TurnOutcome::Rescheduled {
                delivery,
                error: sanitize(&error).chars().take(TurnOutcome::MAX_ERROR_LENGTH).collect(),
            },
            outcome @ (TurnOutcome::Done | TurnOutcome::Cancelled) => outcome,
        };
        let request = CommitRequest {
            address: claim.actor.clone(),
            epoch: claim.epoch,
            message_id: claim.message.id.clone(),
            outcome: outcome.clone(),
            state: decision.state,
            creates: decision.creates,
            sends: decision.sends,
            cancels: decision.cancels,
        };
        let failure = match self.commit_with_retry(claim, guard, request).await {
            CommitEnd::Abandoned => return,
            // A retry commit the store cannot apply (the reschedule's delivery time out of range): the handler's
            // failure stands, with the refusal appended.
            CommitEnd::Refused(reason) => match (&outcome, &decision.handler_failure) {
                (TurnOutcome::Rescheduled { .. }, Some(handler_failure)) => {
                    TurnOutcome::Failed(sanitize(&format!("{handler_failure} (commit refused: {reason})")))
                }
                _ => TurnOutcome::Failed(sanitize(&format!("Commit refused: {reason}"))),
            },
            CommitEnd::Answered(CommitResult::Committed { sent, .. }) => {
                return self.outcome(info, &outcome, &sent).await;
            }
            CommitEnd::Answered(CommitResult::LeaseLost) => {
                tracing::info!("Commit of {} for {} rejected: lease lost", claim.actor, claim.message.id);
                return;
            }
            CommitEnd::Answered(CommitResult::Rejected(reason)) => TurnOutcome::Failed(sanitize(&reason)),
        };
        // Rejected or refused: nothing was applied and the lease is still held; record the failure without effects.
        let request = CommitRequest::new(claim.actor.clone(), claim.epoch, claim.message.id.clone(), failure.clone());
        match self.commit_with_retry(claim, guard, request).await {
            CommitEnd::Answered(CommitResult::Committed { sent, .. }) => self.outcome(info, &failure, &sent).await,
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
    async fn commit_with_retry(&self, claim: &Claim, guard: &LeaseGuard, request: CommitRequest) -> CommitEnd {
        let budget = Instant::now() + self.lease.ttl;
        let mut backoff = COMMIT_RETRY_INITIAL;
        let mut failures = 0;
        loop {
            match self.store.commit(request.clone()).await {
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

    /// After a successful commit: wakes the node for the messages sent (and for a rescheduled message, which may be
    /// due at once), then runs the `on_outcome` hooks.
    async fn outcome(&self, info: &TurnInfo, outcome: &TurnOutcome, sent: &[Message]) {
        if !sent.is_empty() || !outcome.is_final() {
            self.wake.notify_one();
        }
        for plugin in &self.plugins {
            let ctx = OutcomeContext { info, outcome, sent };
            if let Err(e) = guarded(plugin.on_outcome(&ctx)).await {
                tracing::warn!("on_outcome of plugin {} failed; ignored: {e}", plugin.name());
            }
        }
    }
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

/// Runs a hook, turning a panic into an error.
async fn guarded(hook: impl Future<Output = HookResult>) -> Result<(), String> {
    match AssertUnwindSafe(hook).catch_unwind().await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(panic) => Err(format!("panic: {}", panic_message(&*panic))),
    }
}

/// A failure reason every backend can store.
fn sanitize(reason: &str) -> String {
    reason.replace('\0', "\u{FFFD}")
}
