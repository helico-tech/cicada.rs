# Design

Each Factory run has its own time limit, but `settings.timeout` covers the whole type. And Factory reports refused,
failed to start and lost apart from a runtime failure, while a `failed` end tells them apart only by its reason text.
**BREAKING**: the batch becomes 0.21.0 (decision 7, ADR 0062).

```rust
// ContainerJob, next to spec() and inputs():
fn timeout(&self, request: &Self::Request, settings: Option<Duration>) -> Option<Duration> { settings }
pub struct ContainerEnd { pub outcome, pub code, pub kind: Option<FailureKind>, pub reason, pub logs }
pub enum FailureKind { NotStarted, OutOfMemory, RuntimeEnded, Missing, RuntimeErrors }  // camelCase on the wire
pub const OUT_OF_MEMORY: &str = "out of memory";  // the reason of ContainerStatus::Failed for an out-of-memory kill
```

## Decisions

1. **The timeout per request is computed at the create and saved as the deadline,** as the settings' timeout is
   today: the create step asks `timeout`, adds it to the store time after `start` and saves `deadline`. A deadline
   past `MAX_INSTANT` is capped at `MAX_INSTANT` (today it is dropped, which means no limit: the code changes). A
   saved deadline never moves. The job gets the settings' timeout as an argument and returns it by default, so it can
   lengthen, shorten or drop it per request: without the argument a default cannot return the settings' value, and
   `None` could not mean both "use the settings" and "no limit". Not taken: `Option<Option<Duration>>` (hard to
   read); computing it at each poll (a deadline that moves when the job's code changes).
2. **`kind` is set on every new `failed` end and absent on the others.** It is `Option<FailureKind>` with
   `#[serde(default)]`, written as `null` like `code`. An end saved before 0.21.0 decodes with no `kind`, also when
   it failed: old reason text cannot be sorted safely, so no kind is guessed. `ContainerEnd`'s own decoding refuses a
   `kind` on an end that is not `failed`, so a target that decodes a `ContainerEnded` gets the check too, not only
   the state. `FailureKind` is `non_exhaustive` in Rust; on the wire a new kind is a contract change anyway. Not
   taken: a required field (old states would not decode); a kind on every outcome; guessing a kind from old reasons.
3. **Every `failed` path gets one kind** (in `crates/cicada-containers/src/actor/mod.rs`; "inputs" means the create
   step of `container-inputs`):

   | Path | Where | Kind |
   | --- | --- | --- |
   | The key cannot name a container | `step` | `notStarted` |
   | `spec` gives an `Err` | `create` | `notStarted` |
   | `create` fails with `ImageNotFound` or `InvalidSpec` | `create` | `notStarted` |
   | `start` fails with `InvalidSpec` (decision 5) | `create`, `follow` on a `Created` one | `notStarted` |
   | `inputs` gives an `Err`, a file at or above a folder, a bad path | inputs | `notStarted` |
   | `make_dirs`/`copy_in`: `InvalidSpec`, `Conflict` while `Created`; `make_dirs`: `Other` | inputs | `notStarted` |
   | The runtime reports `Failed` with the reason `OUT_OF_MEMORY` | `follow` | `outOfMemory` |
   | The runtime reports `Failed` with any other reason (dead, an error from the daemon) | `follow` | `runtimeEnded` |
   | The runtime reports `Missing` for a started container | `follow` | `missing` |
   | `max_errors` runtime errors in a row | `error` | `runtimeErrors` |
4. **The out-of-memory reason is a constant of the crate.** `OUT_OF_MEMORY` ("out of memory", the text Docker's
   backend gives today) is the reason a runtime puts in `ContainerStatus::Failed` for an out-of-memory kill; the
   rustdoc of `ContainerStatus::Failed` and of the trait and the `container-runtime` spec say so, the Docker backend
   uses it, and the actor matches it exactly. Not taken: a kind on `ContainerStatus::Failed` (a new field in a public
   enum variant breaks every pattern that names its fields); matching free text (a runtime's wording could drift).
5. **A refused start ends at once.** Today `start` failing with `InvalidSpec` counts as a runtime error, and the next
   poll finds Docker's `Failed` status and ends with its reason; that end would be `runtimeEnded`. Now it ends
   `failed`, kind `notStarted`, with the error as reason, as a refused `create` does: one poll earlier, no logs read.
   Limit, in the rustdoc and the handoff: a start refused with another kind (such as `Other`) still counts as an error
   and ends later as `runtimeEnded`, as the status carries no mark of a refused start.
6. **A replayed create step adopts a started container.** The create step now begins with `inspect`. A container of
   that name past `Created` (running or ended) means an earlier step started it and its commit was lost: the step
   saves `started` with a fresh deadline and arms the next poll, without calling `spec`, `inputs`, `create`, the
   copies or `start`, so a refusal cannot end a running container as `notStarted`. `Missing` or `Created`: the step
   goes on as before (build, `create`, the inputs, `start`). This replaces the `inspect` after `create` of
   `container-inputs`: the first `inspect` decides, and a container removed after it shows as a `Conflict` from the
   copies, which `container-inputs` already handles. The calls become `inspect`, `create`, `start` without inputs.
   The fresh deadline counts from the adoption, a little later than the lost start. Not taken: keeping the old step
   and naming the false `notStarted` in the rustdoc (the review's finding stays true).
7. **BREAKING, so 0.21.0.** `ContainerEnd` is in the state and in the end message: the contract hash of every
   container job type changes, and so does that of every target type whose messages take a `ContainerEnded`, and a
   struct literal of `ContainerEnd` needs `kind`. `docs/how-to/install.md` makes public Rust APIs and contract hashes
   compatibility boundaries, and a break raises the minor while the version is 0.x. So the builder renames
   `## 0.20.9 — unreleased` to `## 0.21.0 — unreleased`, sets 0.21.0 in every version place (builder.md, Version),
   and rewrites the section's summary for the whole batch, no longer "Additive". Hosts on 0.20.x follow "Changing a
   contract" in `docs/operations.md` (a versioned type or a maintenance window). ADR 0062 records decisions 2 to 7.
   The test contract `crates/cicada-containers/tests/contracts/test.container.json` is regenerated with
   `CICADA_UPDATE_CONTRACTS=true`. Not taken: a waiver to ship it as 0.20.9 (the rule exists for this case);
   a `non_exhaustive` `ContainerEnd` (hosts' tests build ends by literal; a constructor would cost them more).

## Spec deltas

`container-actor`: *The end is reported once, before removal* (kinds) and *A deadline and errors in a row end a
container* (timeout per request, cap) are modified; *A replayed create step adopts a started container* is added;
`container-inputs`' *Inputs are copied in before start* and *Refused inputs end the container* are modified for the
new first `inspect` (archive `container-inputs` first). `container-runtime`: *A container's status tells its own exit
from the runtime's* is modified (`OUT_OF_MEMORY`).

## Tests

Actor tests on virtual time with `TestActors` and the fake: a request's timeout shorter and longer than the
settings' and none; a deadline capped at `MAX_INSTANT`; one test per row of the table, checking the kind, and that
other ends have none; a refused start ends at once; a replayed step on a running container whose `spec` and `inputs`
would fail adopts it; the new call lists; an old state with a `failed` end and no `kind` decodes; a `kind` on a
`stopped` end is refused, in the state and in a `ContainerEnded`. The wire-format test pins the new contract and
samples with `kind`. Docker's `status_of` test uses `OUT_OF_MEMORY`. No E2E: the actor runs on the fake, and the
Docker backend's reason text does not change.

## Docs it breaks

None of the checked blocks: no README or guide doctest builds a `ContainerEnd` or names the container actor's
timeout. `docs-handoff.md` carries `timeout`, `kind` and its table, `OUT_OF_MEMORY`, the adoption of a started
container, the refused-start limit, and the upgrade to 0.21.0: "Changing a contract" for container job types and
end targets on 0.20.x, and the `ContainerEnd` literal. Install pins move at the release.

## Version

0.21.0 — the unreleased 0.20.9 batch becomes 0.21.0 with this change; released when it has landed.
