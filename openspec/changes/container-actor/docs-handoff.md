# Docs handoff for container-actor

Version: 0.20.7 (the unreleased batch with `container-runtime` and `container-docker`). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` gets a ready-made container actor (ADR 0057), and now depends on `cicada` (no store feature), with
`serde`, `serde_json` and `schemars`. No library crate depends on `cicada-containers`; the gate still checks that.

Public items (all re-exported at the crate root):

- `trait ContainerJob { const TYPE; type Request; fn spec(&self, name: &str, request: &Request) -> Result<ContainerSpec, String>; }`:
  the host names the actor type, as a flow does (ADR 0031). `Request` is serde + `JsonSchema`; its schema is in the
  contract, so the HTTP door checks it. `spec` is sync and runs at each create; the actor sets the spec's name to
  `name` whatever `spec` returns. An `Err` ends the container `failed` with that text and nothing is made.
- `Container::<J>::named(key)`: the typed ref, `<J::TYPE>/<key>`.
- `ContainerActor::new(job, runtime: Arc<dyn ContainerRuntime>, settings)`: the actor type, hosted like any other.
- `ContainerSettings::new(prefix) -> Result<_, InvalidContainer>`, `#[non_exhaustive]` with public fields and these
  defaults: `poll_interval` 2 s, `max_backoff` 60 s, `max_errors` 10, `timeout` none, `stop_grace` 10 s, `log_lines`
  200, `log_bytes` 16 KiB. The prefix starts with a letter or digit and holds only `A-Za-z0-9_.-`.
- `ContainerState::new(request)`: the state to create. Fields: `request`, `goal` (`run`/`stop`/`remove`, null before
  the first message), `phase` (`new`/`started`/`ended`/`removed`), `target`, `deadline`, `end`, `undelivered`, `armed`,
  `due`, `errors`. The state never holds a spec.
- `ContainerMsg`: `Start { to, messageType }` (helper `ContainerMsg::start(&target_ref, "Ended")`), `Stop`, `Remove`,
  `Check`, and `Poll` (the actor's own timer; any poll that is not the armed one is ignored).
- `ContainerEnd { outcome, code, reason, logs }` with `Outcome` = `exited` | `failed` | `timedOut` | `stopped`; the end
  message's payload is `ContainerEnded { key, end }`, so a host's family takes it as `Ended(ContainerEnded)`.
- `Goal`, `Phase`, `EndTarget`, `InvalidContainer`.

Behaviour:

- `Start`, `Stop` and `Remove` only move the goal forward (run → stop → remove, never back) and arm a poll at once.
  The same goal asked again, and `Check`, replace the armed poll with one due when it was (at once when that passed),
  so a lost poll comes back and a backoff holds. With no work left they do nothing.
- Each armed poll takes one step: run/new builds the spec, creates and starts the container (`ImageNotFound` or
  `InvalidSpec` from `create` end it `failed` at once); run/started inspects it (a created one is started again; at
  the deadline the goal becomes stop); stop or remove before the end stops it, inspects it and ends it `stopped`, or
  `timedOut` past the deadline (`stopped` without logs when no container exists); remove after the end removes it.
- The container's name is the prefix plus the key as given (not percent-encoded). A key with other characters, or a
  name over 200 characters, ends `failed` at its first step, without a runtime call.
- A container ends once. The end is saved and sent to the first `Start`'s target in the commit that records it,
  before any removal. A `Start` after the end sends the saved end to that `Start`'s target at once. An end the
  target's type refuses (not registered, or not taking the message) is saved in `undelivered` and the actor goes on,
  so a removal is never blocked. The end message has the ready-made actors' retry policy (7 attempts, 1 s to 32 s).
- Runtime errors never fail a turn: they count in `errors`, and the next poll waits `min(poll_interval × 2^errors,
  max_backoff)`. At `max_errors` in a row, a container without an end ends `failed` ("the runtime failed N times in a
  row: …"); a removal keeps trying at `max_backoff`. A failed `logs` counts the same; at the limit the end gets
  `(no logs: …)`. Any success resets `errors`.
- Logs: NUL → U+FFFD, the last `log_lines` lines, then the last `log_bytes` bytes on a character boundary. A reason:
  NUL → U+FFFD and its first 1 KiB.

Proved by `crates/cicada-containers/tests/actor.rs` (19 tests on virtual time with `TestActors` and `FakeRuntime`,
one per spec scenario, plus the contract and samples), the unit tests in `crates/cicada-containers/src/actor/`, and the
pinned contract `crates/cicada-containers/tests/contracts/test.container.json`. No worked example in `examples/`.

## 1. The container actor in the actors guide

**Where.** `docs/guide/actors.md`, a new section such as `The container actor`.
**Text.** For hosts, with a Rust block modelled on the `ContainerJob` doc example in
`crates/cicada-containers/src/actor/mod.rs` and the harness of `tests/actor.rs`:
- Define a `ContainerJob` (type name, typed request, `spec`), host `ContainerActor::new(job, runtime, settings)`, create
  each actor with `ContainerState::new(request)`, then send `ContainerMsg::start(&target, "Ended")`.
- The target's family takes `Ended(ContainerEnded)`; it gets one end per container, with the outcome, code, reason and
  the tail of the logs.
- Settings and their defaults (above); the prefix rule.
- Never reuse a key, also after a delete: a new actor under an old key adopts the old container and reports its old
  end.
- Send `Remove` before deleting the actor, or the container stays.
- Keep the target alive until the end: a target deleted before it makes the poll's commit fail, which blocks the
  actor until the target exists again (the container still carries the spec's labels for a sweep). A target whose type
  is not registered is not blocking: the end is saved as `undelivered`.
- An image pull (in `create`) and the stop grace run inside one turn: keep the node's `max_turn` above both.
- Secrets go in `spec`, never in the request: the request is stored, the spec is built at each create.
**Caveats.** Polling costs one turn per `poll_interval` for each running container. `Stop` before any `Start`
creates nothing, and a later `Start` gets the `stopped` end.

## 2. The reconciled external resource as a design pattern

**Where.** `docs/guide/design.md`, beside the process example of ADR 0053.
**Text.** A resource with a stable outside name can be reconciled: a message records the goal, one armed poll asks
the outside what is true and takes one step, and the name from the key makes a repeated step find what it made. A
resource without a durable outside identity (a process's PID) needs host-owned handles instead (ADR 0053). The
container actor is the worked case.
**Caveats.** None.

## 3. Placement when only some nodes reach Docker

**Where.** `docs/operations.md`, near the claim owner pinning of ADR 0052.
**Text.** Every node that can claim a container actor must reach the same daemon. When only some do, pin the actors
to those nodes with a claim owner (`CreateOptions::pinned_to`). A node that reaches another daemon sees the container
missing and ends it `failed`, or creates a second one there; the actor cannot detect it.
**Caveats.** None.

## 4. Maps

**Where.** `AGENTS.md`, `Map`, the `crates/cicada-containers` row; and the crate row of `docs/architecture.md`.
**Text.** Add: "the container actor (`actor`): `ContainerJob`, `ContainerActor`, its messages, state and end, and
`actor/text.rs`, the stored-text clean-up; a test job's contract in `tests/contracts`". In the architecture: the crate
now depends on `cicada`, and holds the reconciled container actor of ADR 0057.
**Caveats.** None.

## 5. Skill route

**Where.** `skills/cicada/SKILL.md`: a route to the actors guide's new section, and `cicada-containers` in the
description's crate list. The resolver's `PACKAGES` already holds `cicada-containers` (this change, with its Python
test).
**Text.** "Run containers from actors: `docs/guide/actors.md`, `<the new section>`; `crates/cicada-containers`."
**Caveats.** The route can only resolve once section 1 exists.

## Blocking

None: no doctest, link, guarantee row or skill route breaks.

## Status

Ran `cargo test -p cicada-containers` (35 unit, 19 actor tests, 6 doctests), `cargo test -p cicada-examples --test
skill`, the resolver's Python tests, and `cargo test -p cicada-examples --test docs`. All prose above is proposed text;
none of it is written yet.
