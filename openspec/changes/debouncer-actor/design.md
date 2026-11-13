# Design

## Scope

One trailing debouncer on the public core API, `DebouncerActor`, a unit struct. `Debouncer { name }` with `named`
names `cicada.debouncer/<name>`. Decisions were made with the owner's Opus 5.5 adviser under the owner's instruction
to finish the whole design queue autonomously. Four implementation tasks plus finishing checks suffice.

## Decisions

### One public Flush, one delivery transaction

`DebouncerMsg` has `Submit { to: ActorAddress, message: StoredMessage, retry: Option<RetryPolicy> }` and `Flush`.
Serde uses the crate's tagged/camelCase form; missing/null retry means none. A timer is an ordinary public Flush
with the crate's shared retry policy. Any Flush delivers the current pending value; empty means done with no effects.
There is no sender/id gate. A manual Flush or deliver-now intentionally bypasses the wait; retry/redrive flushes the
current value, not a historical copy. Not chosen: own-only Fire (cannot recover by redrive), or Fire then Deliver
(two turns protect no recurring chain here). This is at-most-one delivery of each kept burst after successful commit,
not idempotency across a new burst: an old Flush redriven later can flush the new burst early.

### The whole latest submission wins

Submit replaces target, stored message, target retry and correlation id together. It does not require the target to
exist or its type to be registered until Flush. A superseded submission ends done and gets no notification.
Not chosen: rejecting a target change, or fixing the target at creation; both complicate last-wins behaviour.
`DebouncerMsg::submit(&target_ref, message, retry) -> Result<Self, EncodeError>` checks the ref and message family.
The target gets the debouncer as sender, a new id, the kept correlation id, kept retry, default retention and ordinary
observer headers from the Flush turn. Incoming headers are not kept. A reply address belongs in the payload.

### State and validation

`DebouncerState` has public `quiet: Duration`, `max_wait: Duration`, `pending: Option<DebouncedMessage>`,
`first_at: Option<Timestamp>` and `armed: Option<MessageId>`. The last three default to None. `DebouncedMessage`
holds `to`, `message`, optional `retry` and optional `correlation_id`. Names are camelCase; durations are ISO-8601.
`DebouncerState::new(quiet, max_wait) -> Result<Self, InvalidDebounce>` creates an empty state.
Construction and serde decoding validate `0 < quiet <= maxWait <= MAX_DELAY`; unknown fields are ignored.
Pending requires firstAt; empty requires firstAt and armed absent. Pending with no armed is valid for recovery.
FirstAt must not exceed MAX_INSTANT. Settings never change by message. `InvalidDebounce` implements Error.
Not chosen: optional maxWait (steady input can postpone forever), updates, counters or a separate config type.
Example initial state: `{"quiet":"PT2S","maxWait":"PT5S"}`.

### Submit and its timer

On Submit, use store time `turn.now()`. If empty set firstAt to now; otherwise keep it, even after a failed Flush.
Replace pending, cancel armed unless it is the current message id, then arm Flush at
`min(now + quiet, firstAt + maxWait)` with the shared retry policy and save its id. All effects commit together.
Use checked time arithmetic; an unrepresentable or unsupported chosen due time fails permanently with no effects.
Core delivery rounds up to microseconds and clamps past due times to store time at commit. maxWait bounds the
scheduled deadline from first handling, not wall-clock execution after mailbox, commit, outage or pause delays.
With quiet 2 s and maxWait 5 s, submissions handled at 0, 1, 2, 3, 4 s deliver the last at 5 s if turns run promptly.
Not chosen: resetting firstAt on each Submit, which would defeat maxWait.

### Flush and failures

With pending, buffer send_raw, cancel armed unless it is the current message id, and clear pending/firstAt/armed.
The send, cancellation and clearing commit together. Store read errors propagate normally; the Flush envelope
policy decides retries. Call-level send rejection is permanent (refused). A missing target rejects the commit
(commit, no retry). Failure leaves the whole state and any other pending timer unchanged; the current failing Flush
itself ends or is rescheduled by the core. Terminal failures notify watchers; the debouncer watches nothing.
A replacement Submit cancels a retrying timer, so the latest value keeps its own quiet deadline.
A target that rejects the delivered payload fails after clearing and does not restore the kept value.

A terminally failed, cancelled or purged timer can leave pending data without a live timer. Manual Flush recovers it;
a new Submit replaces it and rearms using the original firstAt, immediately if maxWait has passed. Cancel/purge do
not notify watchers. Never stop automatically: other submissions might wait in the mailbox. Pause/resume/delete
retain core semantics. Explicit creation and atomic create-and-submit work like the limiter; repeated create keeps state.

### Depth and compatibility

Pending adds three container levels before its payload, so a kept payload can be at most 61 deep. Submit's envelope
can carry 62; if replacing pending would make state too deep, the core's post-handler state check fails the turn
(cause handler), leaving the old pending data and timer intact. The guide names this boundary; no custom recursive
validator or core hook is added. A create with such a state is refused by the existing core state-depth limit.
This is simpler than a second message decoder with a stricter limit that can differ between hosting nodes.
No ADR: an added contract following ADR 0028 changes no existing contract or guarantee, as settled for the scheduler.

## Integration and checks

Pin contract and state/message samples, including empty and pending state, in the actor wire-format tests. Host the
type in the sample service without an automatic instance and update its snapshots/texts. Add a tested example and
actors-guide doctests, guide tables, README, crate docs, architecture crate row, AGENTS map and changelog.
Test runtime scenarios with virtual-time TestActors; reuse the SQLite registry-failure fixture only for store errors.
Cover replacement, quiet/maxWait, public/empty Flush, failures, retry cancellation, redrive, controls, depth rollback,
creation and timestamps. quiet = maxWait is a window from the first handled submission, not an external rate limit.

Review 1 found no blockers. Its note adds both sequence orders of a Submit and Flush with equal delivery timestamps;
the core's `(deliverAt, seq)` order decides whether that submission replaces the old burst or starts the next.
