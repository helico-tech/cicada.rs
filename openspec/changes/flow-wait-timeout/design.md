# Design

## One single-wait API

Add #[must_use] Waited<T> { Received(T), TimedOut }, deriving Debug/Clone/PartialEq/Eq with ordinary T bounds, and
Step::wait_timeout<T:DeserializeOwned+Send>(&mut self,id:&str,request:&Request<T>,timeout:Duration)
returning Result<Waited<T>,HandlerError> asynchronously. Step::wait stays indefinite and returns T. An explicit enum
makes timeout branchable data even when T is Option; an error would incorrectly invoke failure policy. No options
builder, absolute-deadline API, joined waits or target cancellation. Reconcile the two ready, unbuilt predecessors.

ADR 0044's reserved wait fields already hold this behavior; no wire, schema, core or store change. A timeout uses
mode all, one request and until=Some. Preserve all typed structural checks and existing contract hashes.

## Calculate once and replay the saved time

For a new wait, validate id/handle/room and calculate ceil(claim time + timeout) using the sleep wake-time helper,
parameterized only for diagnostic kind. Duration must be <=MAX_DELAY and the rounded result <=MAX_INSTANT. Validate
before looking for an early reply, so invalid timeouts fail even when the reply is ready. One nanosecond rounds up
to one microsecond from a store-microsecond claim; zero gives the claim time. Exactly MAX_DELAY is accepted when the
result fits; MAX_DELAY+1ns is refused. At MAX_INSTANT-1s, 1s fits and 1s+1ns does not. Errors latch permanent failure.

The clock starts at the claim of the turn creating the wait, not the request's earlier turn or the helper's wall time.
The journal stores no request-start time. Replay matches id/kind, exact handle, mode all, one request and a present
until, but ignores the new timeout argument, including an otherwise invalid one. Swapping indefinite/timed APIs,
using a join, another handle, an already-waited request or entry 1001 fails permanently.

## Commit the choice before exposing it

Existing latch and unpolled-future rules apply. At the helper, use these paths:

| Entry and saved replies | Action |
| --- | --- |
| New, reply present | New closed received entry with deadline; arm now. |
| New, no reply | New open entry with deadline; arm At(until). |
| Open, reply present in supplied progress | Close received and arm now, even after until. |
| Open, no reply, claim time >= until | Close timedOut with received=[] and arm now. |
| Open, no reply, claim time < until | Waiting(until): re-arm there without a new entry. |
| Closed received | Decode saved T under InProgress and return Received(T). |
| Closed timedOut | Return TimedOut without decoding or consulting later replies. |

New/Close/Waiting paths suspend; the first latch wins even if flow code catches it or calls another helper. A new
zero-timeout wait without a reply still commits open first, then a later executing Continue decides timedOut.
No data is returned in a deciding turn. Custom Deserialize errors/panics retain the predecessor's permanent latch
behavior and leave the committed outcome intact; fixed code plus Continue can recover a decode mismatch.

Runner effects remain atomic. A close replaces/cancels any older armed id without cancelling the current message,
then arms now with Flow::retry() and executing correlation. A timed open entry uses At(until), so telemetry sees a
self timer; the following arm-now continuation is immediate. The core clamps past At to commit time; this does not
rewrite the journal deadline. Commit or scheduling delays can make the timeout later, never move its saved bound.

## Reply and timer races

Reply turns never inspect the clock. An eligible reply closes an open single wait with or without a deadline,
cancels its armed timer and arms now using the reply's correlation. The deadline is the earliest time an executing
Continue can select timeout, not a cutoff for accepting a reply. Only a flow-executing Continue at/after until with
no saved reply selects timedOut. Mailbox (deliverAt,seq) handling order decides which actually commits first.

With an intact timer, replies delivered before it win; a reply with the same deliverAt is ordered by seq, tested in
both orders. Do not equate send wall time with the winner. If the timer was purged/cancelled, a reply handled after
the deadline still wins. Supplied open progress with a saved reply likewise chooses received before checking time.
A closed timeout ignores later replies before value validation, without state/version changes. It stops no target
work and performs no cleanup. No extra restriction on structurally valid supplied reply records is introduced.

The executing Continue replays earlier entries before reaching the open wait, which is last. Earlier helpers return
saved results; inserting new helper work meets the saved wait and fails replay. Code between helpers remains the
host's deterministic replay code. Later helpers cannot execute after the latch. If a decision commit fails, no choice
was saved, and a reply may win before a later successful timeout commit. A committed choice cannot flip on retry.

## Recovery and rollout

Public Continue keeps normalization (cancel/re-arm, then replay). Before until it re-arms at the saved time; poke,
deliver-now or retry cannot shorten it. A failed/purged/cancelled timer leaves the wait open, with no polling: a reply
can close it, or public Continue recovers timeout processing. Pauses retain mailbox ordering on resume. Correlation
follows the timer chain for timeout, the incoming reply for reply closure, and executing Continue for an early reply.

Joins, Cancel and cancelling remain refused; an eligible reply executing a reserved open join saves nothing and
fails permanently. Existing flow snapshots must equal the landed predecessor's, not the current pre-send source.
Upgrade every host/worker before using deadlines: the predecessor refuses an eligible reply for an open timed wait,
despite matching the hash. Later joins reuse this decision protocol; finalizers calculate separate cleanup deadlines.

Clarify ADR 0044, architecture, actors guide and operations; extend the reply example with a timeout branch and tests.
Verify time boundaries, both equal-time orders, zero/early/supplied replies, failed decisions, purge/pause/recovery,
latches, correlations and timer classification with virtual-time TestActors. Include timeout committed, later step
transiently failed, late reply then retry: still TimedOut, no reissued request. Five tasks plus finishing; no new G/S row.
