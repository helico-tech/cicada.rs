# Design

Picks of the change loop, 2026-11-24, on the proposal's list; decision 3 changes one (its reason is there). Builds on
`container-runtime` (trait, `ContainerStatus`, `ErrorKind`, `FakeRuntime`) and ADR 0057. Reference:
cicada-code-factory `src/actors/container.rs`.

## Decisions

1. **The host names the type, as a flow does** (ADR 0031). `trait ContainerJob { const TYPE; type Request; fn
   spec(&self, name, &Request) -> Result<ContainerSpec, String>; }`, hosted by `ContainerActor::new(job, runtime:
   Arc<dyn ContainerRuntime>, settings)`; the ref is `Container::<J>::named(key)`. Each host brings its own runtime
   and settings. Not a fixed `cicada.container`: one type could not hold two hosts' specs and settings.
2. **The state holds the host's typed `Request`, never a spec.** `ContainerState::new(request)` at the create, like a
   flow's input. `spec` builds the `ContainerSpec` at each create; the actor sets its `name` (decision 5), so secrets
   never reach the store. A typed request puts its schema in the contract, checked at the HTTP door (ADR 0026). `spec`
   is sync: a host loads its secrets before. An `Err` ends the container `failed` with that text, and nothing is made.
   Not a JSON value: no schema, so a wrong request fails only at the first create. Not a copy-in hook: later, additive.
3. **The end goes to a target named in `Start { to, messageType }`, in the batcher's form**: the actor builds the
   payload `{"key": …, "end": ContainerEnd}`, so a host's variant can be `Ended(ContainerEnded)`. This replaces the
   picked `StoredMessage`: its payload is fixed when it is made, and the end is known only later. Merging a field into
   it would make the host build a placeholder end, and `for_ref` could no longer check the variant. `ContainerEnd` is
   `{outcome, code?, reason, logs}`; `outcome` is `exited` (the program's own exit), `failed` (the runtime's: it did
   not start, ran out of memory, went missing, too many errors, a bad name or spec), `timedOut` or `stopped`. The end
   is saved in the state and sent in the commit that records it, before any removal; a container reports one end. The
   first `Start` sets the target; later ones change nothing. No `Start`: the end is saved but not sent. A `Start` after
   the end (a `Stop` came first, or a reused key) sends the saved end to its target at once. A send that `send_raw`
   refuses (a type not registered or not taking the message) is not an error: the end is saved with
   `undelivered: <reason>` and the actor goes on to its goal, so a removal is never blocked. A target that is deleted
   before the end makes the poll's commit fail and blocks the actor until the target exists again: hosts
   keep the target alive until the end (host docs); the container still carries its labels for a sweep.
4. **Reconciled by one armed poll at a time.** `Start`, `Stop` and `Remove` only set the goal (`run`, `stop`,
   `remove`, never backwards) and arm a `Poll` due at once. Only the armed `Poll`, sent by the actor itself, counts. Each
   poll takes one step for (goal, phase), saves it and arms the next:

   | Goal, phase | Step |
   | --- | --- |
   | run, new | `spec`, `create`, `start`; phase `started`, deadline = now + `timeout`; `ImageNotFound` or `InvalidSpec` → end `failed` at once |
   | run, started | `inspect`: running, or created (`start` again) → next poll, or stop at the deadline; `Exited`/`Failed` → end with logs; `Missing` → end `failed` |
   | stop or remove, before the end | `stop` (grace), `inspect`; `Missing` → end `stopped` without logs (nothing was made, or it is gone); else logs, end `timedOut` if the deadline passed, else `stopped`, with the exit code |
   | remove, ended | `remove`; phase `removed` |

   At the deadline the run poll sets the goal `stop`, so the next poll stops the container and ends it `timedOut`.
   `Stop` before any `Start` makes the goal `stop`: its poll calls `stop` anyway (a create whose commit was lost is
   caught), finds `Missing` and ends `stopped`; a later `Start` creates nothing and gets that end. `Remove` with no
   `Start` does the same, then removes. `ImageNotFound` and `InvalidSpec` from `create` are permanent and end at
   once; other errors are never returned
   (a failed poll would be lost): they count in `errors` and the next poll waits `min(interval × 2^errors,
   max_backoff)`; at `max_errors` in a row a container without an end ends `failed` (reason with the last error), while
   a removal keeps trying at `max_backoff`. A failed `logs` counts the same; at the limit the end is saved with
   `(no logs: …)`. `errors` resets when a whole step completes (a start, or a recorded end), not after one call in it. Not one turn that waits: a turn must not hold a claim for a container's
   life.
5. **The name is the settings' `prefix` plus the decoded key** (the key as given, not its percent-encoded form), so a repeated step finds the container. The prefix (required,
   `ContainerSettings::new(prefix)`) starts with a letter or digit and uses only `A-Za-z0-9_.-`. A key with other
   characters, or a name over 200 characters, ends `failed` at its first step without a runtime call. Not an escape or a
   hash of any key: unreadable in `docker ps`, and a host picks its keys anyway.
6. **A lost poll comes back.** `Check`, and a goal asked again (`Start`, `Stop`, `Remove`), cancel the armed poll and
   arm a fresh one due when it was (at once when that passed or none is saved), so a backoff still holds. A goal
   change (`run` → `stop` → `remove`) arms its poll at once instead: the new goal skips the backoff. With no work
   left they do nothing. A fresh poll never starts a second container: decision 5 and the trait's repeat rules.
7. **Settings, defaults by the host**: `poll_interval` 2 s, `max_backoff` 60 s, `max_errors` 10, `timeout` none,
   `stop_grace` 10 s, `log_lines` 200, `log_bytes` 16 KiB; a `#[non_exhaustive]` struct with public fields. The poll's
   retry policy is the ready-made one (7 attempts, 1 s to 32 s). Not settings in the state: the host changes them
   without touching actors; a saved deadline does not move.
8. **Text is made storable where it is read.** Logs: lossy UTF-8 (the trait), NUL → U+FFFD, the last `log_lines`
   lines, then the last `log_bytes` bytes on a character boundary. A reason (runtime or `spec` text): the same, at most
   1 KiB. Not dropping every control character: a terminal's colours are the reader's business.
9. **Placement is the host's.** Every node that can claim an actor must reach the same daemon; when only some do, the
   host pins with ADR 0052's claim owner. Not a pin by the actor: it cannot know the nodes.
10. **The crate depends on `cicada`**, not on `cicada-actors` (decision 3 needs no `StoredMessage`), with `serde`,
    `serde_json` and `schemars`; no feature (both consumers run actors). No library crate depends on it (ADR 0056).

## Contract and tests

The type name is the host's, so nothing goes in `docs/reference/contracts`: as for flows, a test job's contract is
pinned in `crates/cicada-containers/tests/contracts/test.container.json` (`CICADA_UPDATE_CONTRACTS=true` rewrites it),
with samples of every message, the state in each phase and an end payload taken by the schemas and the decoders.
The tests live in `crates/cicada-containers/tests/actor.rs`, with dev-dependencies on `cicada-test` (`TestActors`) and
on the crate itself with `features = ["testing"]` (`FakeRuntime`), so `cargo test --workspace` builds them. Tests on
virtual time with `TestActors` and `FakeRuntime`: a repeated step (the fake already holds the container a lost
commit made) leaves one container and no second `start`; a purged poll comes back on `Check`; the deadline ends
`timedOut`; errors in a row end `failed`, with backoff between polls; `Stop` before `Start` makes nothing and a later `Start` gets the `stopped` end; `Remove` with no `Start` ends without
errors; a missing image ends `failed` at the first poll; an end refused by an unknown target type is saved as undelivered and `Remove` still removes; `Remove`
after the exit records the end with logs before `remove`; a NUL in logs is stored as U+FFFD within the caps.

## Version and gate

Additive: patch, in the unreleased 0.20.7. No guarantee, core wire format, schema or command line change. The gate's
`dependency-trees.sh` keeps checking that no library crate builds `cicada-containers`.

## Docs it breaks

None: no doctest, link, guarantee row or skill route. The builder adds `cicada-containers` to `PACKAGES` in
`skills/cicada/scripts/cicada_source.py` with its Python test (code). For the docs handoff: a section in
`docs/guide/actors.md` (host, start, end message, settings, never reuse a key, Remove before delete, the target
outlives the end, an image pull and the stop grace run inside one turn and its `max_turn`); the reconciled
external resource as a pattern in `docs/guide/design.md`, beside ADR 0053's process example; pinning when only some
nodes reach Docker in `docs/operations.md`; the actor in the AGENTS.md map row and the crate row of
`docs/architecture.md`; a route to the actors guide section in `skills/cicada/SKILL.md`.
