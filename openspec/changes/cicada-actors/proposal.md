# Proposal

## Why

`cicada-extras` holds schedules, a crash-loop guard, a failure watcher that polls outcomes on a timer, and dead
letters. With watches, control effects and child actors in the core, supervision can be an ordinary actor, and the
polling watcher and the guard are no longer needed. ADR 0018 decides on one crate of ready-made actors.

## What Changes

- A new crate `cicada-actors` (layer 3) replaces `cicada-extras`. It uses only the public API of the layers below.
- A ready-made actor has a fixed type name. It stores the messages it sends for others as an address, a message type
  name and a JSON payload. A typed helper builds that from a ref and a message, so a wrong message is still a compile
  error.
- The first three actors:
  - **Supervisor.** It watches actors. When one fails, it restarts or stops it with control effects. The actors it
    starts are its children, so they go when it goes.
  - **Dead letters.** It gets failed messages through a watch, with no polling. An operator redrives or discards one
    with a message.
  - **Topic.** It keeps subscribers and sends each published message to all of them.
- `FailureWatcher` is removed: a watch replaces it. `CrashLoopGuard` went with `turn-observer`, whose claim limit
  replaces it.

The simplest version: three actors. The others wait until someone needs them.

To decide in the design:

- How a supervisor escalates. A failed turn commits nothing, so escalating is a committed turn that sends upward
  (ADR 0018, Left to the changes).
- Schedules. ADR 0018 lists `schedule` under "later", but `cicada-extras` goes away in this change. Either the
  schedule moves here now, or it becomes an example.
- A supervisor restarts actors whose types it does not know at compile time. That needs a create by address and JSON
  state inside a turn, which does not exist. ADR 0018 does not name it; Chorus asks for it too (its change
  `cicada-requests`, point 8). Such a state is JSON that code sends, which nothing checks against the schema
  (`json-schema-at-the-http-door`, Non-Goals): a state that no node can decode fails every message of the actor
  (found when `json-schema-at-the-http-door` was reviewed).
- What a supervisor does with a `Failed` whose error starts with `claim limit:`. A message that kills its node
  takes the messages beside it to the claim limit too (`turn-observer`, decision 10), so a supervisor that restarts
  on every `Failed` would restart healthy actors. The start of the error is a hint and no proof; a field of `Failed`
  for the cause would be proof, and changes the contract of every watcher (found when `turn-observer` was
  reviewed).

## Capabilities

### New Capabilities

- `supervisor`: an actor that restarts or stops the actors it watches.
- `dead-letters`: an actor that collects failed messages and redrives or discards them.
- `topic`: an actor that sends each published message to its subscribers.

### Modified Capabilities

None.

## Out of scope

- Debounce, rate limit, webhook delivery, circuit breaker, request with timeout and scatter/gather: later, each when
  someone needs it.
- Sagas, step workflows, cursor sync, registries and singletons: they stay examples.

## Order

Step 10 of ADR 0018. After `cicada-app`, before `docs-per-layer`. It needs `child-actors`, `watches`,
`control-effects` and `turn-observer`.

## Impact

- `crates/cicada`, if the supervisor needs a create by address and JSON state: the core has none, and
  `control-effects` left it out (found when `control-effects` was reviewed). The design decides.
- A new crate; `cicada-extras` (removed), the examples and the sample service that use it, `README.md`,
  `docs/architecture.md` (the crate table), `docs/operations.md` and `CHANGELOG.md`.
- No change to a guarantee; the contracts of the ready-made actors are new wire formats of their own, covered by ADR
  0018.
- Hosts that use `cicada-extras` must move to `cicada-actors`.
