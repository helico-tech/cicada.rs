# Design

## Use the existing Cancel message

Enable FlowMsg::Cancel, with no new helper, field, contract hash, core or store change. It is sent like any other
message, for example turn.send(&run, FlowMsg::Cancel). A reason field or permission list would expand the wire for
no required behavior. Anyone allowed to send to the run can cancel it; hosts apply their existing HTTP authorization.
No sender check is added. Sending acknowledges enqueueing, not terminal cancellation; callers read the status.

Reconcile the four ready, unbuilt predecessors. Implement this status/message table before Continue normalization:

| Status | Continue | Reply | Cancel |
| --- | --- | --- | --- |
| running | Existing normalization/replay | Existing acceptance/wait behavior | Cancel armed, set cancelled and armed=null. |
| cancelling | Permanent refusal | Permanent refusal | Permanent refusal: needs finalizer release. |
| done | No-op | Ignore before value checks | No-op without state write. |
| cancelled | No-op | Ignore before value checks | No-op without state write. |

Cancel invokes no Flow::run, input clone, step closure or extra typed result decode beyond normal actor loading. It
works before any Continue, with no armed id, a parked wait, missing timer, retry backoff or permanently broken replay.
An undecodable state still fails in the core before handling; Cancel cannot repair it. Do not treat cancelling as
terminal or silently skip its future cleanup: that phase remains explicitly unavailable until the next piece.

## One atomic transition with retained progress

On running, take the saved armed id; if it differs from the current message, buffer turn.cancel(id). Missing or
already-completed ids are harmless under core rules. Set status=cancelled and armed=None. Preserve input, journal,
replies and cleanup exactly. In particular an open main wait stays open, and a sleep's saved time is not rewritten.
The current Cancel completes normally; it must not cancel itself even in trusted supplied progress naming its id.

The cancellation of the armed message, state change and Cancel completion commit together. A failed commit saves
none of them; an unknown acknowledgement does not prove rollback. Reread or resend idempotently. Unapplied commits
after handler return follow lease reclamation/claim limits; handler failures use the sender's retry policy, not
Flow::retry(). No sends,
controls, watches, self-purge or stop are produced. Purging would discard unrelated messages needed by later cleanup;
stop would remove the run. Repeated Cancel/done paths never call state_mut, so state/version stay unchanged.

Cancelled is terminal; Continue cannot restart it. A stalled running run can be recovered by Continue or ended by
Cancel. A new job uses a new run key; there is no un-cancel or permission to reuse external idempotency keys. Existing
requests, children and external resources keep running/existing. No compensation or automatic cleanup is performed.

## Mailbox order, not preemption

All races use actual (deliverAt,seq) handling order. An executing handler can finish and commit new progress before
Cancel is handled; it may already have performed external effects even if its own turn does not commit. Cancel waits
for that turn. Cancelling a running turn through the operator API is a separate core cancellation request, subject
to its normal cooperative/runtime limitations; it may leave the flow stalled, after which FlowMsg::Cancel can end it.

If an armed Continue runs first, Cancel cancels its successor. If Cancel runs first, it cancels the armed work and
later public Continues are no-ops. A Cancel due now can precede a retry in backoff. A reply/deadline decision committed
before Cancel is retained, but its continuation can be cancelled before code consumes that decision. The reverse
order leaves an interrupted wait and ignores the later Reply. Tests use both equal-delivery sequence orders.

Even if the last helper committed, Cancel may win before the final Continue records done. Cancelled does not mean
that no effects ran. Once done commits, Cancel is a no-op. Pauses postpone Cancel until resume; purge can discard a
queued Cancel, leaving the run running. Delayed Cancel is ordinary delayed messaging, not a guaranteed cutoff.
Delete removes the run without running code and purges queued Cancel. Sending to an absent run obeys ordinary send
and commit rejection; create-and-send Cancel together works because creates precede sends.

## Observing and recovering cancellation

A successful Cancel is an ordinary done turn for observers with incoming correlation and no outgoing messages.
The armed message cancelled by it has no handler turn and sends no watch notification. No synthetic Failed or
Stopped is emitted: the run remains readable. Ordinary failures still follow core watch semantics. The state write
can publish a best-effort change hint; no-op paths promise none. Read status after hints, not an assumed event receipt.
An application notification would require explicit application work; there is no automatic completion message here.

An older host refuses Cancel permanently and leaves the run running despite an equal hash. Upgrade every host and
worker before using it. Then resending, operator retry or dead-letter redrive can deliver a new Cancel safely:
identity/from is not relevant to this idempotent command, unlike Reply. Do not equate a failed Cancel with cancellation.

The finalizer piece changes running+Cancel to cancelling followed by cleanup, then cancelled. Runs already cancelled
under this piece are not retroactively finalized. Hosts needing resource cleanup must do it separately or wait for
that feature; retained progress is input for future finalization, not evidence cleanup happened in this release.

## Contracts, docs and tests

Regenerate flow test/sample-service snapshots if public descriptions change. Their hashes and description-stripped
schemas must match the landed join predecessor; other actor/Event contracts remain unchanged. No migration or new
ADR is needed: clarify ADR 0044, architecture, actors guide and operations. The guide distinguishes FlowMsg::Cancel,
pending-message cancel, operator running-turn cancel, pause, purge, delete and Continue, with a compiled example.

Use virtual-time TestActors and counters for new/armed/sleep/parked/open-join runs, partial replies, queued/public
Continues, backoff, long handlers, all terminal/reserved states, broken replay and failed atomic transitions. Test
watcher/observer outcomes, retained targets/children, delete/create and hints without promising delivery. Four build
tasks plus finishing; no user finalizer or core changes.
