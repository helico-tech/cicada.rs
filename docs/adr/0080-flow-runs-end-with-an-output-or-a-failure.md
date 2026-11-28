# 0080. Flow runs end with an output or a failure

Date: 2026-11-28

## Status

Accepted for flow-ends. Partly supersedes [ADR 0031](0031-durable-flows.md) (a continuation out of retries could be
recovered) and [ADR 0044](0044-flow-requests-waits-and-cancellation.md) (a failing cleanup stayed cancelling, and the
cleanup ran only after a `Cancel`).

## Context

A flow run had no result, and a `Continue` that failed for good left it `running` with nothing armed. A caller that
starts a run and waits for it (`flow-request-target`) needs an end it can report: done with a value, cancelled, or
failed.

## Decision

- `Flow` has `type Output`; `run` returns it, and a done run keeps it in its state.
- A run gets the final status `failed` with a reason. An error of the flow code (`run`, `on_cancel`, or a step's error
  they pass on) ends the run as failed when it is permanent or on the last attempt of its `Continue`'s retry policy
  (`failures + 1`, as the core counts, or at once without a retry policy). That turn succeeds: it marks the run
  cancelling and arms its cleanup, as a `Cancel` does, and a successful cleanup ends the run `failed`, not cancelled.
  Cleanup code that fails for good ends the run `failed` at once. The reason is cleaned of NUL and cut at 1,000
  characters.
- The runner's own refusals (replay mismatches, invalid saved state, values that fail the save check), failures while
  applying an entry, killed turns and panics do not end a run: they fail the turn as before, so a fix and a public
  `Continue` recover it.
- A failed run, like done and cancelled, ignores `Continue`, `Reply` and `Cancel` until it is deleted.

## Consequences

- Every `Flow` impl adds `type Output` (`()` for none), and every flow type's contract changes: a `ContractConflict`
  with old nodes, handled by a new type name or a maintenance window (`docs/operations.md`).
- A flow-code failure no longer sends watchers a `Failed` notice and leaves no dead letter: it is the run's status.
- A run that runs out of retries can no longer be recovered by a public `Continue`; a host that wants more patience
  sets `Flow::retry`.
- `flow-request-target` replies with these ends.
