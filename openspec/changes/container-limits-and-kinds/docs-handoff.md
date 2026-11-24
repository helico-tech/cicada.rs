# Docs handoff for container-limits-and-kinds

Version: 0.21.0 (BREAKING; the unreleased 0.20.9 batch became 0.21.0). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

A `failed` container end says why in a machine-readable `kind`, each request can have its own time limit, a replayed
create step adopts a container an earlier step started, a refused start ends at once, and an unsupported runtime call
has its own error kind. Breaking: `ContainerEnd` gains a public field, so every container job type's contract hash
and every end target's hash change (ADR 0062). No README or guide block, link, guarantee row or skill route names the
container actor, so nothing blocks.

- `ContainerEnd::kind: Option<FailureKind>`, on the wire `kind` (camelCase values, `null` when absent, like `code`).
  `FailureKind` (`non_exhaustive`, exported from the crate root): `NotStarted`, `OutOfMemory`, `RuntimeEnded`,
  `Missing`, `RuntimeErrors`. Set on every new `failed` end, never on another; a `failed` end saved before 0.21.0
  decodes with no kind. Decoding an end refuses a `kind` on a non-`failed` end, in the state and in a
  `ContainerEnded`.
- `ContainerJob::timeout(&self, request, settings: Option<Duration>) -> Option<Duration>`, default `settings`.
- `OUT_OF_MEMORY: &str = "out of memory"`, the exact reason of `ContainerStatus::Failed` for an out-of-memory kill.
- `ErrorKind::Unsupported` (display `unsupported`), returned by the trait's default `logs_since`, `copy_out` and
  `make_dirs` instead of `Other`.
- The create step: `inspect` first; past `Created`, adopt; else `spec`, `inputs`, `create`, `make_dirs`, `copy_in`,
  `start`. The `inspect` after `create` of container-inputs is gone.

Proved by the unit tests in `crates/cicada-containers/src/actor/wire.rs`
(`a_failed_end_saved_before_kinds_decodes_without_one`, `a_kind_on_an_end_that_is_not_failed_is_refused`,
`the_failure_kinds_are_camel_case`), `src/error.rs`, `src/fake.rs`
(`a_runtime_without_logs_since_copy_out_or_make_dirs_compiles_and_fails_with_unsupported`), `src/docker/map.rs`
(`status_of` with `OUT_OF_MEMORY`), and by `crates/cicada-containers/tests/actor.rs`
(`a_lost_commit_after_the_start_adopts_the_running_container`, `a_refused_start_ends_not_started_at_once`,
`a_refused_start_of_a_created_container_being_followed_ends_not_started`,
`the_runtimes_failures_of_a_started_container_have_their_kinds`, `each_request_gets_the_timeout_its_job_gives`,
`a_deadline_past_the_largest_instant_is_capped`, `another_error_from_make_dirs_backs_off_and_the_next_poll_starts_it`,
`a_container_gone_after_its_create_backs_off_and_the_next_poll_starts_it`, and kind checks in the existing tests).
The test contract `crates/cicada-containers/tests/contracts/test.container.json` is regenerated. `ContainerJob::timeout`
has a doctest.

## 1. The kind of a failed end

**Where.** The container guide, when the docs session writes it (no guide covers `cicada-containers` yet): the
section on the end of a container.
**Text.** A container ends once: `exited`, `failed`, `timedOut` or `stopped`. A `failed` end also carries `kind`, so a
host can route failures without reading the reason:

| Kind | When |
| --- | --- |
| `notStarted` | The key cannot name a container; `spec` or `inputs` gave an `Err`; an input path is bad or a file is at or above a folder; `create` failed with `ImageNotFound` or `InvalidSpec`; `start` failed with `InvalidSpec`; `make_dirs`/`copy_in` failed with `InvalidSpec`, or with `Conflict` while the container is still `Created`; `make_dirs` is `Unsupported` |
| `outOfMemory` | The runtime reported `Failed` with the reason `OUT_OF_MEMORY` |
| `runtimeEnded` | The runtime reported `Failed` with any other reason (dead, an error from the daemon) |
| `missing` | A started container was gone |
| `runtimeErrors` | The settings' `max_errors` runtime errors in a row |

`exited`, `timedOut` and `stopped` ends have no kind. A host matches on `FailureKind` with a wildcard arm, as it is
`non_exhaustive`.
**Caveats.** A `failed` end saved before 0.21.0 has no kind; none is guessed from its reason. A start refused with
another error kind than `InvalidSpec` (such as `Other`) counts as a runtime error and ends later, usually as
`runtimeEnded`, as the status bears no mark of a refused start.

## 2. A time limit per request

**Where.** The same guide, next to `ContainerSettings::timeout`.
**Text.** `ContainerSettings::timeout` is the type's limit; `ContainerJob::timeout(request, settings)` can lengthen,
shorten or drop it per request (`None` is no limit) and returns `settings` by default. It is asked once, when the
container starts: the deadline is saved then and never moves, also when the settings or the job's code change
later. A deadline past the largest instant is capped at `MAX_INSTANT` (before 0.21.0 it meant no limit). Rust block:
the doctest of `ContainerJob::timeout` in `crates/cicada-containers/src/actor/mod.rs` (`run.limit.map(...).or(settings)`).
**Caveats.** An adopted container (section 3) gets a fresh deadline counted from its adoption, a little later than
the lost start.

## 3. A replayed create step adopts a started container

**Where.** The same guide, the section on the create step and inputs; this corrects the container-inputs handoff.
**Text.** The create step first inspects the container. One past `Created` (running or ended) was started by an
earlier step whose commit was lost: the actor adopts it and follows it, without calling `spec`, `inputs`, `create`,
the copies or `start`, so a refusal by the job now never ends a running container. A missing or `Created` container
goes through the step: `create`, `make_dirs`, `copy_in`, `start`. Without inputs the runtime calls are `inspect`,
`create` and `start`. A container removed between the inspect and the copies makes them fail with `Conflict`; a new
inspect finds it missing, so the error counts and backs off, and the next poll creates it again.
**Caveats.** Two corrections to the container-inputs handoff: the actor no longer inspects after `create`, and a
runtime without `make_dirs` fails with `Unsupported`, not `Other`; `Other` from `make_dirs` (on Docker, any daemon
500) now backs off and retries like any runtime error.

## 4. Out of memory on every runtime

**Where.** The same guide, the section on writing another runtime (or the trait's overview).
**Text.** A runtime reports a container killed for lack of memory as `ContainerStatus::Failed` with the reason
`OUT_OF_MEMORY` ("out of memory"), exactly; the container actor matches it to report `outOfMemory`. The Docker
backend does; another runtime must too. A runtime that does not implement `logs_since`, `copy_out` or `make_dirs`
gets the trait's default, which fails with `ErrorKind::Unsupported`.

## 5. Upgrade to 0.21.0

**Where.** `docs/how-to/upgrading.md`, a new `Upgrade to 0.21.0` section above `Update dependencies for 0.20.6`; a
line in `docs/operations.md`, `Upgrading`, may point to it as for 0.20.0.
**Text.** Version 0.21.0 changes the contract hash of every container job type and of every target type whose
messages take a `ContainerEnded`, as `ContainerEnd` gains `kind`. A node requires exact hash equality, so for each such
type follow `Changing a contract` in `docs/operations.md`: a versioned type name for new runs while old
actors finish on old hosts, or a maintenance window (stop every host of the type, delete its actors and then its
registration, start the new hosts). Send `Remove` to a container actor before deleting it, or its container stays.
In Rust, add `kind` to every `ContainerEnd` struct literal (`kind: None`, or the kind of a `failed` end), such as in a
host's tests, and bind or ignore it in exhaustive patterns. A host that matched on `failed` reason text can match
`kind` instead. The store schema, the HTTP API, the ready-made actors' contracts and the guarantees do not change.
**Caveats.** `ErrorKind` gains `Unsupported`; it is `non_exhaustive`, so a match already has a wildcard arm, but a
host that treated `Other` from the trait's defaults as "unsupported" must match `Unsupported` now.

## Blocking

None.

## Status

Ran `cargo test -p cicada-containers --all-features`, `cargo test -p cicada-examples --test docs` and
`--test skill` (results in the build report). All text above is proposed; no protected prose was edited.
