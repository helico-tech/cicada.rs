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
//! `failed`. The `on_handler_error` hooks then see that decision and may replace it with a final outcome.
//!
//! **After the handler.** Once the handler has returned or failed (or was stopped by its turn timeout), the lease is
//! never released for a re-run (G6): every later failure ends in a `failed` outcome or, when the outcome cannot be
//! committed, in abandoning the turn without release.
//!
//! **Task structure.** Each turn is one task that spawns three children: *work* (load, `on_claim`, the handler under
//! its timeout, hooks, checks), *keeper* (heartbeat and local-deadline watchdog; it aborts *work* when the lease is
//! lost) and *overtime* (a warning about handlers that ignore cancellation). The turn task awaits *work*, then commits
//! while the keeper keeps renewing.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use tokio::sync::Notify;
use tokio::task::AbortHandle;
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;

use super::effects::{EffectBuffer, RawSender};
use super::limits;
use super::resolver::TypeResolver;
use crate::actor::{HostedActor, RunEnd, RunResult, panic_message};
use crate::address::UriTemplate;
use crate::plugin::{
    Attributes, BeforeCommitContext, ClaimContext, HandlerErrorContext, HookResult, OutcomeContext, Plugin, TurnInfo,
};
use crate::store::{
    ActorCreate, Claim, CommitRequest, CommitResult, Delivery, Message, MessageId, OutgoingMessage, RenewResult,
    StateChange, Store, StoreError, TurnOutcome,
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
    /// Node plugins first, then type plugins.
    pub plugins: Vec<Arc<dyn Plugin>>,
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

pub(crate) struct TurnExecutor {
    pub store: Arc<dyn Store>,
    pub resolver: Arc<TypeResolver>,
    pub hosted: HashMap<UriTemplate, HostedType>,
    pub lease: LeaseSettings,
    /// Wakes the node's dispatch loop: after a commit that sent messages, or one that may have applied unacknowledged.
    pub wake: Arc<Notify>,
}

impl TurnExecutor {
    /// Executes one claimed turn to its end. `claim_sent` is the local time at which the claim request was sent.
    /// `cancel` is the node stopping after its grace period (or a dropped `run_due_turns`): the worker is aborted and
    /// waited for; then, unless the handler already ended or the lease was lost, the lease is released.
    pub async fn execute(self: Arc<Self>, claim: Claim, claim_sent: Instant, cancel: CancellationToken) {
        let Some(hosted) = self.hosted.get(&claim.template).cloned() else {
            tracing::warn!("Claimed {} of type {}, which this node does not host", claim.actor, claim.template);
            let _ = self.store.release(&claim.actor, claim.epoch).await;
            return;
        };
        let info = Arc::new(TurnInfo {
            self_address: claim.actor.clone(),
            template: claim.template.clone(),
            message: claim.message.clone(),
            now: claim.claimed_at,
            attributes: Attributes::default(),
            unhandled: std::sync::OnceLock::new(),
            claim_sent,
        });
        let guard = Arc::new(LeaseGuard::new(claim_sent + self.lease.ttl - self.lease.safety_margin()));
        let handler_ended = Arc::new(AtomicBool::new(false));

        let mut work = tokio::spawn(self.clone().decide_or_fail(
            hosted.clone(),
            claim.clone(),
            info.clone(),
            guard.clone(),
            handler_ended.clone(),
        ));
        let keeper = tokio::spawn(self.clone().keep_lease(guard.clone(), work.abort_handle(), claim.clone()));
        let overtime = tokio::spawn(overtime_warning(work.abort_handle(), claim.clone(), hosted.max_turn));

        let decision = tokio::select! {
            biased;
            result = &mut work => match result {
                // The lease was lost while the work ended (it did not yield in time to be cancelled): by now another
                // node may run the turn, so nothing is committed (as a cancelled worker).
                Ok(_) if guard.is_lost() => None,
                Ok(decision) => Some(decision),
                Err(e) if e.is_cancelled() => None,
                Err(e) => {
                    let phase = if handler_ended.load(Ordering::SeqCst) { "after its handler ended" } else { "before its handler ran" };
                    let panic = e.try_into_panic().map(|p| panic_message(&*p)).unwrap_or_default();
                    tracing::error!("Turn of {} for {} panicked {phase}: {panic}", claim.actor, claim.message.id);
                    Some(Decision::failed(format!("Turn failed {phase}: panic: {panic}")))
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
                } else if handler_ended.load(Ordering::SeqCst) {
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
            Some(decision) => self.commit(&hosted, &claim, &info, decision, &guard).await,
        }
        keeper.abort();
        overtime.abort();
    }

    /// The keeper: renews the lease and aborts `work` when it is lost.
    async fn keep_lease(self: Arc<Self>, guard: Arc<LeaseGuard>, work: AbortHandle, claim: Claim) {
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
                    Ok(RenewResult::Renewed(_)) => {
                        guard.extend(sent + self.lease.ttl - self.lease.safety_margin());
                        wait = self.lease.heartbeat;
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
        handler_ended: Arc<AtomicBool>,
    ) -> Decision {
        match AssertUnwindSafe(self.decide(&hosted, &claim, &info, &guard, &handler_ended)).catch_unwind().await {
            Ok(decision) => decision,
            Err(panic) => {
                let phase = if handler_ended.load(Ordering::SeqCst) {
                    "after its handler ended"
                } else {
                    "before its handler ran"
                };
                let panic = panic_message(&*panic);
                tracing::error!("Turn of {} for {} failed {phase}: {panic}", claim.actor, claim.message.id);
                Decision::failed(format!("Turn failed {phase}: panic: {panic}"))
            }
        }
    }

    async fn decide(
        &self,
        hosted: &HostedType,
        claim: &Claim,
        info: &Arc<TurnInfo>,
        guard: &LeaseGuard,
        handler_ended: &AtomicBool,
    ) -> Decision {
        // Load: state, own ref, message.
        let loaded = match hosted.actor.load(claim) {
            Ok(loaded) => loaded,
            Err(reason) => return Decision::failed(reason),
        };
        if let Some(reason) = &loaded.unhandled {
            let _ = info.unhandled.set(reason.clone());
        }

        for plugin in &hosted.plugins {
            let mut ctx = ClaimContext { info };
            if let Err(e) = guarded(plugin.on_claim(&mut ctx)).await {
                return Decision::failed(format!("onClaim of plugin {} failed: {e}", plugin.name()));
            }
        }

        // Run.
        let started = Instant::now();
        let ended = tokio::time::timeout(hosted.max_turn, (loaded.run)(info.clone(), self.resolver.clone())).await;
        // Also after the turn timeout: the outcome is failed(timeout), never a re-run.
        handler_ended.store(true, Ordering::SeqCst);
        // A handler that only blocked its thread past max_turn returns its value: judge the duration ourselves.
        let overran = started.elapsed() > hosted.max_turn;
        match ended {
            // Cut off at max_turn: a handler failure, retried under the message's policy, seen by the error hooks.
            Err(_) => {
                let failure = HandlerFailure::Timeout { max_turn: hosted.max_turn, cause: None };
                self.on_handler_error(hosted, info, failure, "timeout".into(), true).await
            }
            // It ended, but only after max_turn (blocking code): still a timeout. Re-running is allowed only when the
            // handler itself failed; one that returned, vetoed or decided ends failed(timeout), not retried.
            Ok(run) if overran => {
                let cause = match run.end {
                    RunEnd::Threw(f) => Some(Box::new(f)),
                    _ => None,
                };
                let retryable = cause.as_ref().is_some_and(|c| !c.is_permanent());
                let failure = HandlerFailure::Timeout { max_turn: hosted.max_turn, cause };
                self.on_handler_error(hosted, info, failure, "timeout".into(), retryable).await
            }
            Ok(run) => match run.end {
                RunEnd::DecidedFail(reason) => {
                    tracing::info!("Unhandled policy of {} failed {}: {reason}", info.self_address, info.message.id);
                    Decision::failed(reason)
                }
                RunEnd::DecidedDrop => Decision::of(TurnOutcome::Done),
                RunEnd::Threw(failure) => {
                    let reason = failure.reason();
                    self.on_handler_error(hosted, info, failure, reason, true).await
                }
                RunEnd::Returned => self.after_return(hosted, info, run, guard).await,
            },
        }
    }

    /// The handler failed; its effects are discarded. First the dispatcher decides from the message's retry policy;
    /// then every `on_handler_error` hook runs on fresh effects and sees that decision. A failing hook is ignored
    /// together with what it changed.
    async fn on_handler_error(
        &self,
        hosted: &HostedType,
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
        let retry =
            retry_delay.map(|d| TurnOutcome::Rescheduled { delivery: Delivery::After(d), error: reason.clone() });
        let mut effects = EffectBuffer::default();
        let mut outcome = retry.clone().unwrap_or_else(|| TurnOutcome::Failed(reason.clone()));
        for plugin in &hosted.plugins {
            let mark = effects.mark();
            let before = outcome.clone();
            let mut ctx = HandlerErrorContext {
                info,
                sender: RawSender { resolver: &self.resolver, effects: &mut effects, allow_reserved: true },
                error: &failure,
                outcome: outcome.clone(),
                retry: retry.clone(),
                retry_delay,
                failure_reason: reason.clone(),
            };
            let result = guarded(plugin.on_handler_error(&mut ctx)).await;
            let changed = ctx.outcome;
            match result {
                Ok(()) => outcome = changed,
                Err(e) => {
                    effects.roll_back(&mark);
                    outcome = before;
                    tracing::warn!("onHandlerError of plugin {} failed; ignored: {e}", plugin.name());
                }
            }
        }
        Decision {
            outcome,
            state: StateChange::Unchanged,
            creates: effects.creates,
            sends: effects.sends,
            cancels: Vec::new(),
            handler_failure: Some(reason),
        }
    }

    async fn after_return(&self, hosted: &HostedType, info: &TurnInfo, run: RunResult, guard: &LeaseGuard) -> Decision {
        let mut effects = run.effects;
        for plugin in &hosted.plugins {
            let mut ctx = BeforeCommitContext {
                info,
                sender: RawSender { resolver: &self.resolver, effects: &mut effects, allow_reserved: true },
            };
            if let Err(e) = guarded(plugin.before_commit(&mut ctx)).await {
                return Decision::failed(format!("beforeCommit of plugin {} failed: {e}", plugin.name()));
            }
        }
        match self.check_typed_effects(info, &effects, guard).await {
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
                    if let Err(e) = limits::check_json("state", state.as_ref()) {
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

    /// Checks what the compiler cannot: that every typed ref the handler created or sent to resolves, through the
    /// TypeRegistry, to the type of its ref; and a create's initial state against the created type.
    async fn check_typed_effects(
        &self,
        info: &TurnInfo,
        effects: &EffectBuffer,
        guard: &LeaseGuard,
    ) -> Result<Option<String>, StoreError> {
        for create in &effects.creates {
            let what = format!("type of {}", create.address);
            let resolved = self.retry_read(guard, &what, || self.resolver.resolve(&create.address)).await?;
            let Some(resolved) = resolved else {
                return Ok(Some(format!("Create of {}: no registered actor type matches it", create.address)));
            };
            if resolved.registered.template() != &create.template {
                return Ok(Some(format!(
                    "Create of {}: the address belongs to type {}, not {}",
                    create.address,
                    resolved.registered.template(),
                    create.template
                )));
            }
            let state = create.state.as_ref().filter(|s| !s.is_null());
            if resolved.registered.contract.state_schema().is_none() && state.is_some() {
                return Ok(Some(format!(
                    "Create of {}: {} is stateless and takes no state",
                    create.address, create.template
                )));
            }
            if let Some(target) = self.hosted.get(&create.template)
                && let Err(e) = target.actor.check_state(state)
            {
                return Ok(Some(format!("Create of {}: the initial state cannot be decoded: {e}", create.address)));
            }
        }
        for (address, template) in &effects.typed_targets {
            if address == &info.self_address {
                continue;
            }
            let what = format!("type of {address}");
            // No such instance either: the commit rejects the send.
            let Some(resolved) = self.retry_read(guard, &what, || self.resolver.resolve(address)).await? else {
                continue;
            };
            if resolved.registered.template() != template {
                return Ok(Some(format!(
                    "Send to {address}: the address belongs to type {}, not {template}",
                    resolved.registered.template()
                )));
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

    async fn commit(
        &self,
        hosted: &HostedType,
        claim: &Claim,
        info: &TurnInfo,
        decision: Decision,
        guard: &LeaseGuard,
    ) {
        // Every backend can store the reason (PostgreSQL refuses NUL characters in text). A rescheduled message carries
        // its `lastError` along, so that one is also bounded.
        let outcome = match decision.outcome {
            TurnOutcome::Failed(e) => TurnOutcome::Failed(sanitize(&e)),
            TurnOutcome::Rescheduled { delivery, error } => TurnOutcome::Rescheduled {
                delivery,
                error: sanitize(&error).chars().take(TurnOutcome::MAX_ERROR_LENGTH).collect(),
            },
            TurnOutcome::Done => TurnOutcome::Done,
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
            // A retry commit the store cannot apply (the reschedule's delivery time out of range, or invalid data in a
            // hook's effects): the handler's failure stands, with the refusal appended.
            CommitEnd::Refused(reason) => match (&outcome, &decision.handler_failure) {
                (TurnOutcome::Rescheduled { .. }, Some(handler_failure)) => {
                    TurnOutcome::Failed(sanitize(&format!("{handler_failure} (commit refused: {reason})")))
                }
                _ => TurnOutcome::Failed(sanitize(&format!("Commit refused: {reason}"))),
            },
            CommitEnd::Answered(CommitResult::Committed { sent, .. }) => {
                return self.outcome(hosted, info, &outcome, &sent).await;
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
            CommitEnd::Answered(CommitResult::Committed { sent, .. }) => {
                self.outcome(hosted, info, &failure, &sent).await
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
    async fn outcome(&self, hosted: &HostedType, info: &TurnInfo, outcome: &TurnOutcome, sent: &[Message]) {
        if !sent.is_empty() || !outcome.is_final() {
            self.wake.notify_one();
        }
        for plugin in &hosted.plugins {
            let ctx = OutcomeContext { info, outcome, sent };
            if let Err(e) = guarded(plugin.on_outcome(&ctx)).await {
                tracing::warn!("onOutcome of plugin {} failed; ignored: {e}", plugin.name());
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
