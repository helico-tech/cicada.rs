# Design

The fourth change of the container-outputs line. At a container's end the host keeps the rest of its log and an output
folder in its `OutputStore`, and the owner learns what was stored and what was skipped. Factory does this one container
after another in its runner, so one slow copy holds up the others (Factory issue 0005).

```rust
let actor = ContainerActor::new(Tests, runtime, settings).with_store(store.clone());
impl ContainerJob for Tests {
    fn outputs(&self, _: &Test) -> Option<Outputs> { Some(Outputs::new("/work/out")) }   // limits: 64 MiB, 1000 files
    // ...
}
// The owner's end: {"key": "t1", "end": {"outcome": "exited", "code": 0, …, "outputs": {"stored": 12, "bytes": 40960,
//   "skipped": 1, "firstSkipped": [{"path": "/work/out/core", "reason": "…"}], "error": null}}}
```

## Decisions

1. **In the actor's end step, so each container collects on its own.** Container actors run their turns in parallel
   (up to the node's concurrency), so collecting in each actor's own turns is the fix for issue 0005: a slow copy holds
   only its own actor. **A new phase `Collecting`, so a saved end still means a sent end.** For an end the container
   ran for (`exited`, `failed` with `runtimeEnded` or `outOfMemory`, `stopped`, `timedOut`), an actor with a store
   puts the recorded end (outcome, code, kind, reason, logs) into a new state field `pending` and the phase
   `Collecting`, with `end` still empty; it collects in its next steps, and the turn that finishes sets `end` to the
   pending end with its `outputs`, the phase `Ended`, and sends it with `container-progress`'s last progress, read in
   that turn. Without a store, an end is recorded and sent in one commit, as today. A `Start` during `Collecting`
   keeps the first target and gets the end when the collection finishes; a `Stop` does nothing more; a `Remove` sets
   the goal and the container is removed after the end is sent (no cut-short collection). A state saved by 0.22 that
   is `Ended` never collects and sends no second end. The state's validity rule allows `pending` only in
   `Collecting`. The spec's rule "The end is reported once, before removal" is modified to say this (design review
   round 2). The rustdoc of `logs_since`, `copy_out` and `LogArchiver` changes from "outside
   actor turns" to "in a turn, a long call needs the job's `MAX_TURN`". Not taken: a collector task outside turns (the
   end would wait for it through messages or polling; two places own one end).
2. **Opt-in twice: a store on the actor, outputs on the job.** `ContainerActor::with_store(Arc<dyn OutputStore>)`
   gives the actor a store; `ContainerJob::outputs(&self, request) -> Option<Outputs>` (default `None`) names the
   folder and limits (`Outputs::new(path)`: 64 MiB and 1000 files, `CopyLimits` of `copy_out`). Without a store nothing
   changes and `outputs` is absent. With a store, `outputs` is always present: with no outputs folder only the log is
   drained and the counts are 0, so an error after too many failures has a place.
3. **The log first, a few pages per turn, through the archiver's write.** Each collection turn reads `log_end` and
   pages `logs_since` from its cursor with `append_logs(container, after, page)`, at most 20 pages (10 000 lines), then
   arms an immediate next poll while pages stay full, so no turn reads a whole log. Next to a running `LogArchiver` it
   never repeats or loses a line: an `OutputError` of kind `Conflict` re-reads `log_end`. This closes the archiver's
   gap for a container removed between rounds. `Unsupported` skips the log.
4. **Then the folder, file by file, into the store, in one turn.** One turn does `copy_out(name, path, limits)` and
   `put_file` for each copied file, in order, so "the counts of the last attempt" are that turn's; a repeat replaces
   the same files (a retried step is safe). A file the store refuses (`Invalid`, such as a control character in its
   name) goes into the skipped list with the store's reason, and the rest go on; `copy_out`'s own skipped entries join
   that list. The report is the new optional field `outputs` of `ContainerEnd`: `OutputsReport { stored, bytes,
   skipped, firstSkipped, error }`, with `firstSkipped` the first 10 `SkippedFile { path, reason }` (the type of
   `copy_out`; a size appears only in a reason) and `error: Option<String>`. With the new phase and state fields, a
   contract change of every container job type, shipped with `container-progress` in 0.23.0; ADR 0066 records it.
   `ContainerEnd` is not `#[non_exhaustive]`, so Rust code that builds it changes too (the upgrade handoff). Memory
   per container is at most the byte limit: the rustdoc says to size it against the concurrency.
5. **Errors retry like runtime errors, then deliver the end as recorded.** A `copy_out`, `logs_since` or store error a
   retry can cure (`DaemonUnavailable`, `Other`, `OutputError::Unavailable`/`Other`) counts as a runtime error and arms
   the next poll with the backoff. At `max_errors` in a row the end is delivered with its recorded outcome unchanged
   and `outputs.error` = `(no outputs: …)`; the spec's rule of errors in a row is modified to say so (`runtimeErrors`
   stays for a container without a recorded end). `InvalidSpec` (a bad outputs path) and `Unsupported` from
   `copy_out` end the collection at once with that error; a `ContainerError` of kind `Conflict` (the container is gone)
   delivers the end with
   what is stored. The container is removed only after the end is delivered, as today.
6. **The store's input rules get tighter** (from the code review of `container-output-store`): a container name over
   the actor's 200 characters, and a control character (NUL included) in a path part, give `Invalid`; the rustdoc of
   `OutputStore` warns that a store on a case-insensitive file system must not map `Job` and `job` to one folder.

## Tests

With `FakeRuntime`, `MemoryOutputStore` and `TestActors` on virtual time: an exited container's rest of log and output
files land in the store and the end carries the report; a file over the limit is skipped with its reason; a file the
store refuses is skipped and the others are stored; more than 10 skipped give `skipped` > 10 and 10 listed; no store
leaves `outputs` absent and a store without an outputs folder gives counts of 0; a `Start` and a `Remove` during
`Collecting` get the end once, after the collection; a 0.22 `Ended` state collects nothing; a long log is drained across
several turns; a running archiver and
the end step store each line once; a `DaemonUnavailable` copy retries and then succeeds; a store `Unavailable` that
comes back; `max_errors` failures deliver the end with its outcome and the error; `InvalidSpec` and `Unsupported` end
the collection at once; a removed container delivers what is stored; two containers, one slow, end independently; a
0.22 end JSON without `outputs` decodes; the tightened input rules in `check_output_store`; a contract snapshot.

## Docs

The handoff asks for the guide's outputs section (store on the actor, `outputs` on the job, limits and memory, the
report, `MAX_TURN`) and the 0.23.0 upgrade section shared with `container-progress` (contracts, the Rust literal of
`ContainerEnd`).
