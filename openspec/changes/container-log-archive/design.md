# Design

The second change of the container-outputs line. A host keeps each container's log in its `OutputStore`
(`container-output-store`), following it while the container runs and draining it once it ended, so the log survives
the container's removal. Factory does this in its runner today.

```rust
let archiver = LogArchiver::new(runtime.clone(), store.clone(), "ci-");   // the job's container prefix
App::new("factory").task("logs", move |_system, stopping| async move {
    archiver.run(async move { stopping.wait().await }).await;
    Ok(())
});
```

## Decisions

1. **The actor labels its containers with its prefix.** `ContainerActor` adds the label `cicada.container.prefix` =
   the settings' prefix to every spec it creates (after the job's `spec`, so a job cannot drop it). The archiver finds
   a job's containers with `list` of that label, and `container-orphan-alert` uses it too. An adopted container made
   before this change has no label and is not archived; the handoff says so. Not taken: reading the actors' states
   from the store (heavier, and the archiver needs no `ActorSystem`); the host labelling in `spec` (every host would
   repeat it).
2. **A `LogArchiver` outside turns, run by the host, one per Docker daemon.** `LogArchiver::new(runtime, store, prefix,
   settings)` with `ArchiveSettings::new()` (interval 2 s; `page_lines` 500, refused outside 1 to `LOG_PAGE_MAX` with an
   `InvalidArchive` error from `new`; `max_parallel` 16 reads at once) and `async fn run(self, stop: impl Future<Output
   = ()>)`, which returns when `stop` completes. A host runs it as an `App::task` (`app-tasks`) or spawns it itself;
   `cicada-containers` does not depend on `cicada-app`. Each archiver repeats every log scan of its daemon, so a host
   runs one per Docker daemon; two on one daemon stay correct (decision 6) but cost twice. Not taken: the container
   actor reading logs in its turns (a page is too big for actor state, and a slow log would hold a turn).
3. **Rounds never wait.** Each round lists the containers of the label and starts a read for each container that is not
   drained and not still being read from an earlier round, up to `max_parallel` reads at once; the round does not
   wait for them, so a slow log holds up only itself. A read takes `log_end` from the store, calls `logs_since` from
   its cursor until a short page, and appends each page with `append_logs(container, after, page)`; an empty page
   whose cursor did not move is not written. A running container is read again in a later round; an ended one is
   drained once (read to a short page after its end) and then kept in an in-memory set of drained names, which each
   round prunes to the names `list` returned; after a restart each ended container is drained once more, which finds
   nothing new.
4. **Errors keep the archiver going.** A failed `list` is logged and tried again next round. From `logs_since`:
   `Unsupported` marks the container drained and is logged once for it (its log can never be read, or the runtime
   has no `logs_since`); `Conflict` (the container is gone) drops it; every other kind, those added later included,
   leaves it for a later round, and after 5 failed reads in a row it waits `interval × 2^(n−5)`, at most 60 s, before
   the next. From the store: `Conflict` (another writer moved the cursor) re-reads `log_end` next round; `Unavailable`
   and `Other` count as failed reads; `Invalid` is logged once and skips the container. Nothing panics or ends `run`.
   Logging uses `tracing`, a new dependency of `cicada-containers` (already in the workspace).
5. **Only containers that exist are followed.** The archiver follows what `list` gives, so it never writes for a removed
   container; `container-output-retention` removes outputs only after the container is removed, so a re-archive from
   the start (the cursor is `None` again after `remove`) cannot happen. Read-back is `OutputStore::read_logs`: no
   helper of its own.
6. **The end hands over through the store.** `container-output-collect` drains the rest of the log at the end as part
   of its work through the same guarded append, so the two cannot double lines: whichever writes first moves the
   cursor, and the other's `Conflict` makes it re-read.

## Tests

With `FakeRuntime` (timed log lines) and `MemoryOutputStore`, on virtual time: a running container's lines arrive in the
store across rounds, each once, in order; an ended container is drained and skipped afterwards; a restart (a new
archiver on the same store) adds nothing; `Unsupported` marks it drained; a removed container is dropped; a scripted
`DaemonUnavailable`, `InvalidSpec` and a failed `list` are retried, with the backoff after 5; two archivers on one store
store every line once; a container whose read hangs does not hold up the others, and is not read twice at once;
`page_lines` 0 and 1001 are refused; `stop` ends `run`. The actor adds the label, also when the job's spec sets other
labels.

## Docs

Nothing breaks. The handoff gives the container guide the archiver, the label, running it as a task (one per Docker
daemon), and the limits (containers made before the label are not archived; the last lines of a container removed
between rounds are lost until `container-output-collect`).
