# Proposal

## Why

`cicada-extras` holds a schedule helper, a failure watcher that polls outcomes on a timer, and dead letters; its
crash-loop guard went with `turn-observer`. With watches, control effects and child actors in the core,
supervision can be an ordinary actor, and the polling watcher is no longer needed. ADR 0018 decides on one crate of
ready-made actors.

## What Changes

- **BREAKING** A new crate `cicada-actors` (layer 3) replaces `cicada-extras`. It uses only the public API of the
  layers below.
- **BREAKING** A `Failed` says why its message failed. It holds a cause: the handler failed, the handler refused
  the message, a timeout, a message or a state that cannot be decoded, a rejected commit, a failure of the store
  or the node around the turn, or the claim limit. Its schema is part of the contract of every type that watches,
  so each such type gets a new contract.
- A ready-made actor has a fixed type name (`cicada.supervisor`, `cicada.dead-letters`, `cicada.topic`). A message it
  keeps to send later is a type name and a JSON payload; a typed helper builds that from a ref and a message, so a
  wrong message is still a compile error.
- The first three actors:
  - **Supervisor.** It creates its children, watches them, and restarts one whose message failed through a fault of
    the child: its handler failed or its turn ran too long. A child whose state its type cannot read is given up and
    shown. A message that is itself at fault, a rejected commit, a failure of the store and the claim limit restart
    nothing. One incident is one
    restart, however many queued messages fail. After too many restarts it gives up on the child and fails a message
    of its own, so that a supervisor above it restarts it; a child it cannot start at all is given up without a
    restart above. Its children are fixed by its state, and go when it goes.
  - **Dead letters.** It gets failed messages through a watch, with no polling. An operator redrives or discards one
    with a message.
  - **Topic.** It keeps subscribers and sends each published message to all of them, each in a turn of its own, and
    drops a subscriber that is not there at its next delivery.
- `FailureWatcher` is removed: a watch replaces it. `CrashLoopGuard` went with `turn-observer`, whose claim limit
  replaces it. `Schedule` moves to `cicada-actors` as it is.
- The core gets `Turn::create_raw` and `Turn::create_child_raw`: a create by address with a JSON state. A supervisor
  needs it, and Chorus asks for it (its `cicada-requests`, point 8).

The simplest version: three actors, one way to supervise (restart the child that is at fault). The others wait
until someone needs them.

## Capabilities

### New Capabilities

- `supervisor`: an actor that restarts the actors it watches when they are at fault.
- `dead-letters`: an actor that collects failed messages and redrives or discards them.
- `topic`: an actor that sends each published message to its subscribers.
- `create-by-address`: a turn creates an actor by its address, with a state given as JSON.

### Modified Capabilities

- `watches`: a `Failed` says why the message failed.
- `outcomes-listing`: its requirement about the failure watcher goes.

## Out of scope

- Debounce, rate limit, webhook delivery, circuit breaker, request with timeout and scatter/gather: later, each when
  someone needs it.
- Sagas, step workflows, cursor sync, registries and singletons: they stay examples.

## Order

Step 10 of ADR 0018. After `cicada-app`, before `docs-per-layer`. It needs `child-actors`, `watches`,
`control-effects` and `turn-observer`.

## Impact

- A new crate; `cicada-extras` (removed); `crates/cicada` (`Failed` and the cause a commit carries, in the node and
  in all three stores; the two creates by address); the examples and the sample service that use the
  old crate; `e2e` (its compose file names `FAILURE_WATCHER_INTERVAL`); the workspace manifest, `AGENTS.md` (the
  map), `README.md`, `docs/architecture.md` (the notifications, the crate table, the calls of a turn),
  `docs/operations.md` and `CHANGELOG.md`.
- The wire format changes: `Failed` gains a field. ADR 0028 records that, where ADR 0023 fixed its schema, and how
  a supervisor escalates, which ADR 0018 left open. `TurnOutcome`, which an observer gets, does not change.
- Hosts that use `cicada-extras` move to `cicada-actors`, and a host with a type that watches changes that type's
  contract. Chorus and Karakuri have neither.
