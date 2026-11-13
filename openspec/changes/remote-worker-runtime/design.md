# Design

## Worker and source

Add runtime::worker and root/runtime exports Worker, WorkerBuilder, WorkerConfig, WorkerError, TurnSource and
SourceError. Worker is cloneable shared state. Its builder(source: impl TurnSource) has host, observe, config and
build like ActorSystem; no feature flag or new dependency. The async-trait, object-safe TurnSource is Send + Sync:
register_contract(&Contract), list_types(), lease_turns(owner, name/hash map, limit), renew_turn(actor, epoch),
release_turn(actor, epoch), complete_turn(actor, epoch, message_id, report). Each returns the predecessor's core
result (list_types returns Vec<TypeDef>, release returns ()), with SourceError. Implement for ActorSystem and Arc<T>.
No wire types or public Backend; the ActorSystem adapter uses its public operations and types().definition values.

SourceError is cloneable Unavailable(String), Invalid(String) or Fatal(String). For the reference adapter, map
OpError::Invalid/Store(Invalid) to Invalid, Store(Unavailable/Corrupt) to Unavailable, and Start to Fatal. An error
does not authorize release after handler end. Adapters implement the core lease/complete contract, including
nonshortening renewal, unique claims and explicit Rejected reports; they do not hide retries of mutation requests.

WorkerConfig defaults: random worker_id, poll_interval 500ms, default_max_turn 15min, concurrency 16,
shutdown_grace_period 30s and shutdown_cancel_timeout 5s. No worker TTL, heartbeat override, claim limit or sweep.
Expose config(), state()->NodeState, running_turns(), hosted_types(), initialize()/start()->Result<(), StartError>,
run_due_turns(limit)->Result<usize, WorkerError>, stop(), stop_with_grace(Duration) and
stopped()->Option<WorkerError> (async, waits for Stopped). WorkerError is Start(StartError), Source(SourceError) or
ContractMismatch { type_name, registered: Option<String>, requested }. Explicit stop yields None; retain the first
automatic-stop error for every waiter. The worker never stops or closes its source.

## Initialize and dispatch

Validate positive poll/default_max_turn/concurrency, representable duration deadlines and the predecessor's owner
limits; zero shutdown waits are allowed as for the node. Validate hosted definitions/duplicates and at most 100
types. Register exact generated contracts and require returned stored hashes to match; map conflict to existing
StartProblem::ContractConflict and other failures to Other. Failed initialization is retryable; success caches the
host map/hashes, not registry query results. An empty worker initializes/starts but never sends an empty lease request.

start is one-shot (New only). Registration awaits hold no lifecycle lock that blocks stop: stop can mark a New
worker stopped; start checks state again before spawning. Late registration may have persisted contracts but starts
no handler. initialize itself does not dispatch. Concurrent starts serialize the final New-to-Started transition.

Reserve free concurrency slots before each lease request and retain reservations through the detached call and
handoff; running_turns counts execution tasks, not reservations. Poll for min(free, MAX_LEASE_BATCH); full batches
self-wake immediately, sparse/empty/unavailable rounds wait poll_interval or a completion wake. No due-time query,
push listener or node row. run_due_turns shares the reservations with polling/other manual calls, makes at most one
batch request capped by limit/free/100, and returns the number of turn tasks started after they settle. Zero/free-zero
returns 0 without leasing. It works in New or Started; Stopping/Stopped returns a WorkerError::Start state problem.

Unavailable from a manual lease call is returned, not retried there; the poll loop backs off. Source Invalid/Fatal
or ContractMismatch on either lease path records the first error and triggers graceful stop outside the calling
turn/dispatch task, avoiding self-join. A Fatal on completion abandons that turn; on renewal it cannot extend a
deadline. These operations do not themselves stop the whole worker; a subsequent failing lease request does.

## Timing and executor reuse

Reuse TurnExecutor with private Holder::{Store, Source}, per-turn LeaseTiming(sent, ttl), and report output from its
work task. Local mode derives Decision and uses its existing commit/retry/fallback path; remote mode calls the source
with the report. Reconcile with the landed predecessor's helper layout. HostedActor still loads the private Claim;
convert TurnLease to it without losing any message fields. Worker disables the executor's local claim-limit branch.

Store mode keeps its configured heartbeat and timing. Source mode uses heartbeat=max(ttl/3, 1ns), with grants below
2ns or overflowing checked deadlines treated as invalid source metadata. No 1ms floor: legal server TTLs can be
smaller. Deadline is sent+ttl-min(ttl/10,1s); renew from its own request start and keep max(old confirmed, candidate).
Use the latest confirmed TTL for the next heartbeat, renew immediately when less than a heartbeat remains, and
retry renewal errors after min(heartbeat/4,1s), floored at 1ns. The independent watchdog still runs while renew hangs;
lease loss or keeper panic aborts work. Late grants start no observer/load/handler and get best-effort release.
Before accepting an extension, atomically check loss and expiration of the old confirmed deadline; an expired grant
cannot revive even if renewal and watchdog become ready together. Check the same boundary before completion starts.

Validate response batch size, requested types, duplicate actors within the batch, message.to/actor agreement and
timing before handing it to executors; a malformed batch is a Fatal lease error with guarded best-effort cleanup.
TurnSource remains a trusted implementation of lease exclusivity, not an untrusted-server security boundary.
Turn::now remains claimed_at; current_time keeps claimed_at + elapsed since worker request start. It is an estimate
and can lead store time by request transit and server admission/processing time, not merely network delay.

TypeResolver gets Registry::{Store, Source}; registry lookup stays per call, retaining the full-JSON parsed-contract
cache from ADR 0036. Map source Invalid to StoreError::Invalid and Unavailable/Fatal to StoreError::Unavailable so
raw sends/creates keep TurnError and Node cause behavior. Share handler-phase arbitration, MAX_TURN/overrun handling,
encoding/decode/panic paths, typed post-handler checks, watch/control buffering, observer headers and overtime logic.
Factor only common task spawn/cancel/drain mechanics; retain ActorSystem's dispatch and manual scheduling behavior.

## Completion retries

Remote completion retains current_report. On Completed use the actual outcome/sent envelopes to wake and end the
worker observer, even when a Returned report became failed. On Rejected accept a valid nonretryable failure and
replace the report permanently. Failed(retryable=true) can legitimately be rejected while rescheduling, so it must
also switch. Rejected for an already nonretryable Failed report is a protocol error: abandon without release.

Capture one budget_end = completion_start + latest confirmed ttl, checked for overflow; never reset it after a
Rejected transition or renewal. Retry Unavailable with 50ms exponential backoff capped at 2s and the budget/lease.
Each pending source.complete_turn is raced against that fixed budget and the guard's loss signal, so a hanging call
cannot keep a slot/keeper forever. Dropping that future is uncertain. Invalid/Fatal, malformed results or LeaseLost
abandon without release; log possible application after unknown results. The keeper continues during completion.
Guard expiry cannot be extended after loss, and no further completion starts once lost or the budget is exhausted.
Only Completed ends observers; no message read or retention-hidden outcome proves an uncertain commit succeeded.

## Cleanup and verification

Detached lease calls return a guarded batch through a oneshot. If the receiver disappears or stop wins handoff,
release known leftovers best effort and return reservations. A lost response may contain unknown leases that only
expire. Dropping a manual batch cancels only its cohort; all its turns remain tracked until actual cleanup, so another
batch cannot bypass concurrency. Worker stop drains manual tasks even in New; local ActorSystem behavior is unchanged.

Stop polling, wait at most cancel_timeout for dispatch, then grace for tracked turns, cancel and wait cancel_timeout.
Non-yielding work may outlive stop. Release only after work stopped and before handler end; after handler end, finish
within the remaining completion budget or abandon. Operator cancellation received through renewal that beat handler
end completes Cancelled, including if shutdown follows. Shutdown/manual cancellation alone releases stopped unfinished work.
No cleanup success is promised through a failed/closed source. stopped waiters are notified without missed wakes.

Five tasks suffice: source API, executor/resolver seams, Worker lifecycle, parity/fault tests, docs; then finishing.
Tests cover local/remote parity and every timing/handoff/fallback boundary, including in-flight hung completion,
retryable-failure rejection, simultaneous renewal/watchdog readiness, completion-start expiry, operator cancellation
followed by shutdown, mixed manual/poll capacity and stop during registration. ADR 0039 records this extension.
