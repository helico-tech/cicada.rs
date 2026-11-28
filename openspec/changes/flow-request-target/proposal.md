# Proposal

## Why

Factory (on 0.25.0) has reusable flows that several callers start and wait for: `delivery.verify` (called by a flow),
`delivery.revision` and `tickets.review` (called by an actor), `security.threat-triage` (by an actor and by hand).
Today a flow cannot be the target of a request: `FlowMsg` is only `Continue | Reply | Cancel`, so a request cannot
carry a `ReplyTo` into a run; the input is fixed in the initial state, built before the request id exists; a run has no
result (`Flow::run` returns `Ok(())`); and a run has no failed end (a `Continue` that fails for good leaves it running
with nothing armed). So Factory writes these parts as actors with explicit states. The owner agreed on 2026-11-28 (F1).

## What Changes

- A run can be started with a reply route: a new `FlowMsg::Start { reply }`. The input stays in the created state, so
  a flow caller writes `step.request(id, Target::child(&run, &initial), FlowMsg::start)`. The run saves the first route
  and ignores later ones, then arms `Continue` as a host start does.
- When the run ends, it replies to the saved route with its outcome: done with the output, cancelled after its cleanup,
  or failed with a reason. The output and the failed end come from `flow-ends`, the first half of the split.
- A flow caller waits on `Request<FlowOutcome<O>>`; an actor caller gets the outcome as a message.

To decide in the design: the outcome's wire shape; the route as `ReplyRoute<FlowOutcome<O>>` (from `reply-route`); what
a run without a route does at its end (nothing); and a `Start` that reaches a run that has already ended.

## Out of scope

- Waiting for a run that was started without a route (a host `Continue`).
- Changing how steps, sleeps, sends or waits replay.
- A work id or header carried through runs: `work-id-header`.

## Order

After `flow-ends`. Before `work-id-header`.

## Impact

Crates: cicada-actors.

`crates/cicada-actors/src/flow.rs` and its tests, the test flow's contract in `crates/cicada-actors/tests/contracts`.
The contract of every flow type changes (a new message variant and state fields): a `ContractConflict` with old nodes,
handled as `docs/operations.md` describes. Released in the same version as `flow-ends`, so flow contracts break once.
An ADR and an upgrade note. Docs handoff: the actors guide's flow sections, the upgrade guide,
and the design guide's text on requests.
