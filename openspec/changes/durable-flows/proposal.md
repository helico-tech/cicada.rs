# Proposal

## Why

Multi-step jobs repeat a state machine: steps, result fields and self-sends, as in
[examples/src/steps.rs](../../../examples/src/steps.rs). A replaying flow can put their order in ordinary async code
while retaining one commit per completed step. The core already provides actors, atomic state/effects and retries.
The original proposal included sleeps and event starts; these are now separate small changes after this runner.

## What Changes

- Add `Flow`, `FlowActor<F>`, typed `FlowRun<F>`, `FlowState<Input>` and `Step` in layer 3 on the public core API.
  The host supplies the flow's actor type name, an explicit exception to the crate's fixed ready-made type names.
- Each run is one actor. Public Continue starts or recovers it by arming a self-Continue with the flow's retry policy.
  Only the currently armed message executes the flow, so callers cannot create competing continuation chains.
- `step.run(id, closure)` returns a saved JSON result during replay, or executes one new closure, records its result,
  arms the next Continue and suspends. A private latch preserves suspension and failures even if user code catches
  the returned error. The flow returns HandlerResult; ordinary `?` propagates the control result.
- Save an ordered journal, enforce stable unique step ids and result decoding, and fail changed replay permanently.
  Results can be null. Input and journal stay readable through ordinary actor reads; done runs remain until deletion.
- Give each closure a stable `<run address>/<step id>` idempotency key. Execution before a successful commit can repeat;
  external effects remain at least once. Code outside closures must be deterministic and free of external effects.
- Define the journal's Step and Sleep wire variants in the first release. The next change adds the sleep API without
  altering this contract. Keep step count and depth bounded; document host flow evolution and contract snapshots.
- Add an ADR and a sibling onboarding-flow example; preserve the explicit state-machine example. Host the example in
  the sample service and document ordinary App hosting and explicit create-and-Continue.

The owner delegated remaining design choices to Opus 5.5 and authorized finishing the full queue. The split was agreed
with that adviser: this runner, then `flow-sleep`, then `flow-events`; each remains one behaviour.

## Capabilities

### New Capabilities

- `flows`: replay saved results and execute one new external step per actor turn, with durable progress and recovery.

### Modified Capabilities

None.

## Out of scope

- Durable sleep's public API (next change) and starting runs through event actors (the change after that).
- Step sends to actors, wait-for-event, invoke, parallel steps, cancellation compensation, concurrency controls,
  automatic run removal, large results outside state and migration of in-flight runs to changed flow code.
- New endpoints, special App hosting methods and core changes.

## Order

After `batcher-actor`, before `flow-sleep`.

## Impact

`crates/cicada-actors` (flow module, runtime and wire tests), an ADR and its index row, a new example and its tests,
actors-guide doctests, README, crate docs, architecture, AGENTS map, changelog, and sample-service hosting/contracts.
No existing actor contract changes. Hosts who do not use flows are unaffected. No host has requested this yet;
Chorus's existing workflows remain a possible future fit, not a dependency of this design.
