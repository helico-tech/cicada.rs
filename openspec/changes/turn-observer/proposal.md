# Proposal

## Why

The `Plugin` trait has three hooks, three context types and a typed key-value bag (`Attributes`), and a plugin may
refuse a turn at claim. It has two real users: the telemetry of `cicada-otel` and `CrashLoopGuard` in `cicada-extras`.
ADR 0018 replaces it with one narrow observer.

## What Changes

- **BREAKING** `Plugin`, `Attributes`, the three hook contexts, `ActorSystemBuilder::install` and `Turn::attributes`
  are removed.
- A new trait, `TurnObserver`: it is called when a turn starts, may add headers to the messages the turn sends, and
  returns a value that is told the outcome when the turn ends. A value dropped without an outcome means the turn was
  abandoned.
- An observer cannot refuse a turn and cannot change what it commits.
- `cicada-otel` implements `TurnObserver` and stays its own crate, so the core does not depend on OpenTelemetry. The
  core itself only writes `tracing` events.
- Crash loops get an answer in the core, because `CrashLoopGuard` worked by refusing claims. ADR 0018 names a claim
  limit that fails the message; such a message then ends `failed` and a watch sees it.

The simplest version: one trait with one method and one returned value.

To decide in the design:

- The claim limit: what it counts, its default, and whether a sender or a type can set it. Chorus asks for a cap on
  lost leases too (its change `cicada-requests`, point 6).
- Whether a node takes one observer or several.

## Capabilities

### New Capabilities

- `turn-observer`: code that watches turns start and end, without a say in them.
- `claim-limit`: a message whose turns keep dying ends `failed`.

### Modified Capabilities

None.

## Out of scope

- A metrics API in the core.
- Moving OpenTelemetry into the core.

## Order

Step 7 of ADR 0018. After `control-effects`, before `json-schema-at-the-http-door`. It comes after `watches` so that
a message failed by the claim limit reaches a watcher.

## Impact

- `crates/cicada` (`plugin` becomes the observer, `runtime`, `turn`), `cicada-otel`, `cicada-extras` (the guard is
  removed), about twenty test plugins in `cicada-test`, the sample service, `README.md`, `docs/architecture.md` (the
  term *plugin*, turn semantics), `docs/operations.md` (telemetry) and `CHANGELOG.md`.
- ADR 0018 supersedes the three plugin hooks of ADR 0012; no further ADR.
- A claim limit fails a message whose handler never returned. The design says which row of the turn semantics in
  `docs/architecture.md` states that.
