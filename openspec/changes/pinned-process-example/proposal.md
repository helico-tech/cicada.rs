# Proposal

## Why

Pinning selects which executor handles an actor, but it does not retain a child-process handle across turns or make
process spawning atomic with a Cicada commit. The original pinned-actors proposal asked for a checked example of
these boundaries. Under the owner's autonomous delegation on 2026-11-16, separate that example from the store/API
change so each can be reviewed and built in a small change.

## What Changes

- Add a worked actor/host example that starts, observes and stops a local child process on its pinned node.
- Keep live handles in the host and desired/observed state plus operation/incarnation identity in actor state.
- Define repeat handling, an outside effect followed by a lost commit, stale exit notifications and host restart.
- Check start/stop, natural exit, repeated attempts and restart behavior with deterministic tests and a small real
  process smoke test where supported. Hand user guidance and skill/example routes to the docs workflow.

To decide in the design: process identity and repeat detection; host restart/reconciliation policy; how the watcher
sends into ActorSystem; resource limits and cleanup; what the runnable adapter can actually guarantee after an
abrupt host death. Never promise generic exactly-once process spawn or PID-only adoption.

## Out of scope

Extending supervisor/flow/event protocols to create pins; the example creates its pinned actor directly.

Changing the pin API/claim rule. A production process manager, container orchestrator, automatic actor migration,
websocket connections or fencing arbitrary external programs. Node-down notifications remain the next change.

## Order

After `pinned-actors`. Before `node-down-alert`.

## Impact

Examples, example-only dependencies if necessary, tests, skill routes, ADR if the example needs a recorded contract,
changelog and docs-handoff.md. No new core API or store schema. The docs session owns handwritten user guidance.
