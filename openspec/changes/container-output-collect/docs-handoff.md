# Docs handoff for container-output-collect

Version: 0.23.0 (BREAKING; in the unreleased 0.23.0 section opened by container-progress). Refresh the dependency pins
of `docs/how-to/install.md` once it is published.

## What changed

With an output store, the container actor keeps the rest of a container's log and its job's output folder there at the
end, in its own turns, and the end tells the owner what was stored and skipped (ADR 0066). New and changed public items
of `cicada-containers`:

- `ContainerActor::with_store(Arc<dyn OutputStore>) -> Self`.
- `ContainerJob::outputs(&self, request) -> Option<Outputs>`, default `None`.
- `Outputs { path, limits }` (`#[non_exhaustive]`), `Outputs::new(path)`: 1000 entries and 64 MiB.
- `ContainerEnd` gains `outputs: Option<OutputsReport>` (wire `outputs`, written only when present).
- `OutputsReport { stored: u32, bytes: u64, skipped: u32, first_skipped: Vec<SkippedFile>, error: Option<String> }`
  (wire `stored`, `bytes`, `skipped`, `firstSkipped`, `error`).
- `SkippedFile` gains serde and `JsonSchema` (`{"path": …, "reason": …}`).
- `Phase::Collecting` (wire `collecting`, between `started` and `ended`); `ContainerState` gains
  `pending: Option<ContainerEnd>` (wire `pending`). A state has `pending` exactly in `collecting`, and no `end` there.
- `check_container_name` refuses a name over 200 characters; `clean_output_path` refuses a control character (NUL
  included). `MemoryOutputStore` and `check_output_store` follow. The rustdoc of `OutputStore` has a **Case** paragraph.
- The rustdoc of `logs_since`, `copy_out` and `LogArchiver` no longer says "outside actor turns": called in a turn, a
  long call needs the job's `MAX_TURN`.

Proven by `crates/cicada-containers/tests/outputs.rs` (one test per scenario of the design's Tests section), the unit
tests in `src/actor/wire.rs` (`an_end_without_outputs_decodes_and_keeps_none_on_the_wire`,
`an_inconsistent_state_is_refused`) and `src/outputs/mod.rs`, `check_output_store`, the test job's contract snapshot
`tests/contracts/test.container.json`, and the doc examples of `ContainerJob::outputs` and `ContainerActor::with_store`.

## 1. Outputs at the end

**Where.** `docs/guide/containers.md`, under `## The container actor`, a new section such as `### Outputs at the end`,
after `### Progress` (container-progress's handoff) or after `### The end`.
**Text.** A host keeps what a container leaves behind, the rest of its log and an output folder, in its own
`OutputStore`, and the owner learns with the end what was kept. Two opt-ins:

```rust
let actor = ContainerActor::new(Tests, runtime, settings).with_store(store.clone());

impl ContainerJob for Tests {
    // ...
    fn outputs(&self, _: &Test) -> Option<Outputs> {
        Some(Outputs::new("/work/out")) // at most 64 MiB in 1000 files
    }
}
```

(compiled in the doc examples of `ContainerActor::with_store` and `ContainerJob::outputs` in
`crates/cicada-containers/src/actor/mod.rs`; `store` is an `Arc<dyn OutputStore>`, such as the one a `LogArchiver`
writes.)

- **What happens.** At an end the container ran for (`exited`, `failed` with `runtimeEnded` or `outOfMemory`, `stopped`,
  `timedOut`), the actor holds the end as `pending` in the phase `collecting`. Its next polls store the rest of the log
  (at most 10 000 lines per turn, the next poll at once while more is left), then copy the outputs folder out within
  its limits and put each file into the store, keyed by the container's name. The poll that finishes reads the last
  progress, records the end with `outputs` and sends it. A saved end still means a sent end.
- **The report.** The owner's end gains `outputs`:
  `{"stored": 12, "bytes": 40960, "skipped": 1, "firstSkipped": [{"path": "/work/out/core", "reason": "…"}], "error": null}`.
  `skipped` counts every entry left by the limits and the link rule of `copy_out` and every file the store refused (a
  control character in its name, for example), each with its reason; `firstSkipped` lists the first 10.
- **Without an outputs folder**, only the log is stored and the counts are 0. **Without a store**, nothing changes: the
  end is recorded and sent in one commit and has no `outputs`. Ends the container did not run for (`notStarted`,
  `missing`, `runtimeErrors`) never collect.
- **Errors.** A runtime or store error a retry can cure counts as a runtime error and backs off; at `max_errors` in a
  row the end goes out with its outcome unchanged and `error: "(no outputs: failed N times in a row: …)"`. A relative
  outputs path, a runtime without `copy_out` and a container removed by someone else end the collection at once with
  the error; a runtime without `logs_since` skips the log. What was stored before an error stays stored.
- **Messages during the collection.** A second `Start` changes nothing: the end goes to the first target when the
  collection finishes. A `Stop` stops nothing more. A `Remove` removes the container after the end is sent.
- **Next to a `LogArchiver`.** Give both the same store: the guarded append stores each line once, and the actor stores
  the lines the archiver had not read when the container was removed.
- **Turn limit and memory.** The copy runs in one turn: set the job's `MAX_TURN` above the time a copy of the byte limit
  takes. A collecting container holds up to its byte limit in memory: size the limits against the node's concurrency.
  Each actor collects in its own turns, so a slow copy holds up only its own container.

**Caveats.** The store is the host's: the actor does not delete stored outputs (retention is a later change). A store on
a case-insensitive file system must keep `Job` and `job` apart.

## 2. Store input rules

**Where.** `docs/guide/containers.md`, the section on the output store (`OutputStore`, its input rules).
**Text.** Add: a container name over 200 characters, and a path with a control character (NUL included), give
`Invalid`. A store on a case-insensitive file system (the defaults of macOS and Windows) must not map `/out/Job` and
`/out/job`, or the containers `Job` and `job`, to one file or folder.

## 3. Upgrade to 0.23.0, continued

**Where.** `docs/how-to/upgrading.md`, `## Upgrade to 0.23.0` (from container-progress's handoff).
**Text.** Add to the contract paragraph: the state also gains `pending` and the phase `collecting`, and `ContainerEnd`
the optional `outputs`. **An owner type whose message family takes `ContainerEnded` changes its contract too**, as the
end's schema gains `outputs`: follow `Changing a contract` for those types as well.

In Rust, add `outputs: None` to every `ContainerEnd` struct literal (it is not `#[non_exhaustive]`), `pending: None`
where you build a `ContainerState` by its fields, and `Phase::Collecting` to exhaustive matches on `Phase`. A host's own
store that uses `check_container_name` and `clean_output_path` now refuses names over 200 characters and paths with a
control character.

**Do not roll back.** A container actor with a store saves states in the phase `collecting`, which 0.22 cannot read: do
not roll a node back to 0.22 while any is.

## 4. Rustdoc wording the guide may repeat

**Where.** `docs/guide/containers.md`, wherever it says that `logs_since`, `copy_out` or the `LogArchiver` run "outside
actor turns".
**Text.** The archiver runs as a task of its own; the actor with a store calls `logs_since` and `copy_out` in its turns,
where a long call needs the job's `MAX_TURN`. The data goes to the store, not into actor state.

## Blocking

None.

## Status

Ran `cargo test -p cicada-examples --doc` (45 passed), `--test docs` (11 passed) and `--test skill` (18 passed) on this
branch: no guide block builds `ContainerEnd` or matches `Phase`, so nothing blocks. All of the above is proposed text;
no prose is written yet.
