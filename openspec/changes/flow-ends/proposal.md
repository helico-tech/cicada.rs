# Proposal

## Why

`flow-request-target` lets a caller start a flow run and wait for its outcome. That needs two things a run lacks
today: a result, because `Flow::run` returns `Ok(())`; and a failed end, because a `Continue` that fails for good leaves
the run running with nothing armed. This change is the first half of the split of `flow-request-target` (my call under
the owner's A1, 2026-11-28): how a run ends. The second half, `Start { reply }` and the outcome reply, follows.

## What Changes

- `Flow` gains `type Output`, and `run` returns it. It is saved in the run's state when the run is done. A flow without
  a result writes `type Output = ();` and keeps returning `Ok(())`.
- A run gets the status `failed` with a reason. An error of the flow code ends its main code when it is permanent, or
  on the last attempt the `Continue`'s retry policy allows: that turn starts the cleanup (`on_cancel`) as a `Cancel`
  does, and the run ends `failed` after it. Cleanup code that fails for good ends the run `failed` at once. The runner's own refusals (replay mismatches, broken state) do not end a run, so a fixed
  deploy and a public `Continue` still recover it.
- A failed run, like a done or cancelled one, ignores `Continue`, `Reply` and `Cancel` and stays until deleted.

## Out of scope

- Starting a run with a reply route, and replying with its outcome: `flow-request-target`.
- A turn killed by the maximum turn time, or a panic: they keep today's behaviour (the run stays in its phase, and a
  public `Continue` recovers it).

## Order

After `reply-route`. Before `flow-request-target`.

## Impact

Crates: cicada-actors, cicada-examples, cicada-sample-service, cicada-otel.

**BREAKING**: every `Flow` impl adds `type Output`; `FlowState` gets an output type parameter and `FlowStatus` a variant;
a flow-code failure no longer sends watchers a `Failed` notice (it is the run's `failed` status); and the contract of
every flow type changes (a new status value and two state fields), a `ContractConflict` handled as `docs/operations.md`
describes. ADR 0080 (partly superseding ADR 0031 and ADR 0044) and an upgrade note. Released together with
`flow-request-target`.
`crates/cicada-actors/src/flow.rs` and its tests, the test flow's contract in `crates/cicada-actors/tests/contracts`,
the flow examples in `examples/`, the sample service's contract snapshot, `cicada-otel`'s flow test. Docs handoff: the actors guide's flow sections and the upgrade guide.
