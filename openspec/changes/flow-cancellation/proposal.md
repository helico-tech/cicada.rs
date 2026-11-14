# Proposal

## Why

A running flow can only be stopped by deleting its actor. A delete runs no handler, so the flow gets no chance to
clean up: work it started in other actors or outside the store keeps running, and nothing reports when it has
stopped. The durable-flows change left cancellation out on purpose.

Found in cicada-code-factory: a run of its work owns containers, networks and a volume. When a person stops a ticket,
the run must stop starting stages, stop the containers that run, save what they produced, remove the resources, and
only then say "cancelled", because capacity is given back at that moment. Today a hand-written actor does this
(`docs/proposals/development-workflow-actors.md` of that repository).

## What Changes

- A public cancel request for a run. It is safe to repeat, and a run that is done or already cancelled ignores it.
- From that request on the run starts no new work: no new step, send or wait.
- An open sleep or wait (`flow-reply-waits`) is interrupted, and the flow's code sees the cancellation at that point.
- The flow then runs its own finalization: it asks the actors and resources it started to shut down and waits for
  their answers, durably, with the same steps and waits as before. A restart in the middle continues the cleanup.
- Only when finalization has finished does the run report `cancelled`, as a status of its own beside `done`, with a
  watch notification like any other stop.
- A delete stays what it is: immediate, with no cleanup. Cleanup outside the store stays the host's job.
- This is more than one small change. The design loop splits it, for instance: (1) the cancel request, the status and
  no new work; (2) interrupting a sleep or a wait; (3) finalization that can step and wait.

## To decide

- How the flow's code sees a cancellation: an error value from the interrupted step, sleep or wait that the flow
  handles, or a separate finalizer function on `Flow`.
- A step that is running when the request arrives: it finishes and commits first (the proposal), or its handler is
  interrupted.
- Finalization that fails for good, or never ends: the run's status then, and whether a second cancel or a deadline
  forces the end.
- Whether a cancel reaches the children of the run by itself, or only through the flow's own finalization.
- What the journal holds for the request and the cleanup against the 1,000 entries, and how replay stays the same for
  a run that was cancelled half-way.
- The flow's contract: a new message and a new status. In-flight runs of older code must keep replaying.

## Capabilities

### New Capabilities

- `flow-cancellation`: a run can be asked to stop, cleans up durably, and reports `cancelled` when that is finished.

### Modified Capabilities

- `flows` (the status, the messages), `flow-sleep` and `flow-waits` (interruption).

## Out of scope

Compensation of finished steps (a saga's undo); a forced stop with a deadline that kills a running handler; cleanup of
anything outside the store by the library; pausing and resuming a run; any change to delete or to watches.

## Order

After `flow-reply-waits`, the last of the line.

## Impact

`crates/cicada-actors` (`flow`, its contract in `docs/reference/contracts` and `tests/contracts`), an ADR beside
ADR 0031, the actors guide and its doctests, an example, the changelog. No store or core change is proposed. Once it
is released, cicada-code-factory can stop a run and give back its capacity only after its containers are gone.
