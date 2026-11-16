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

Decided under the owner's delegation (2026-11-16; Opus 5.5 reviews): one actor per never-reused run, 32 retained
host slots with one spawn each, a fresh host epoch separate from stable node identity, and explicit reconciliation
rather than PID adoption or automatic restart after host death. The inert child mode of the example executable
keeps the OS adapter inspectable. Virtual-time actor/protocol tests and separate bounded OS tests establish distinct
claims. No production process manager or generic exactly-once spawn guarantee.

## Out of scope

Extending supervisor/flow/event protocols to create pins; the example creates its pinned actor directly.

Changing the pin API/claim rule. A production process manager, container orchestrator, automatic actor migration,
websocket connections or fencing arbitrary external programs. Node-down notifications remain the next change.

## Order

After `pinned-actors`. Before `node-down-alert`.

## Impact

Examples, example-only dependencies if necessary, tests, skill routes, ADR 0053 recording the bounded example contract,
changelog and docs-handoff.md. No new core API or store schema. The docs session owns handwritten user guidance.
