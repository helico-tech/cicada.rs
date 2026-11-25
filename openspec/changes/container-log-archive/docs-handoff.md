# Docs handoff for container-log-archive

Version: 0.22.1 (the unreleased batch that `container-output-store` started). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` keeps container logs in the host's `OutputStore` by itself:

- `LogArchiver::new(runtime, store, prefix, settings) -> Result<LogArchiver, InvalidArchive>` and
  `LogArchiver::run(self, stop: impl Future<Output = ()>)`, in `src/archive/`. `LogArchiver` is `Clone`.
- `ArchiveSettings::new()` (also `Default`), with public fields `interval` (2 s), `page_lines` (500) and
  `max_parallel` (16). `new` refuses a `page_lines` outside 1 to `LOG_PAGE_MAX` with `InvalidArchive`, which prints
  why.
- `PREFIX_LABEL`, the label `cicada.container.prefix`, which `ContainerActor` now puts on every container it creates,
  with its settings' prefix as the value, after the job's `spec`.
- `tracing` is a new dependency of the crate, and `tokio` is no longer optional (it was one only with `docker`).

Proven by the tests of `crates/cicada-containers/src/archive/tests.rs` (on virtual time with `FakeRuntime` and
`MemoryOutputStore`: rounds, pages, restarts, removal between rounds, a hanging read, two archivers on one store, every
error rule, the backoff and the settings check), by the doc examples of `LogArchiver` and `ArchiveSettings`, and by
`the_actor_labels_its_containers_with_its_prefix_after_the_job_spec` in `crates/cicada-containers/tests/actor.rs`.

## 1. Archive the logs

**Where.** `docs/guide/containers.md`, the section on keeping outputs that `container-output-store` handed off (such as
`## Keep outputs`), a subsection such as `### Archive the logs`; or right after it.
**Text.** A host does not write the log pages itself: a `LogArchiver` follows the log of every container of a container
job and keeps it in the host's `OutputStore`, so the log survives the container's removal and a host's route can serve
it with `read_logs`. It runs outside actor turns, as a page is too big for actor state. Run it as an app task:

```rust
let archiver = LogArchiver::new(runtime.clone(), store.clone(), "ci-", ArchiveSettings::new())?;
let app = App::new("factory").task("logs", move |_system, stopping| async move {
    archiver.run(async move { stopping.wait().await }).await;
    Ok(())
});
```

(`no_run`; `runtime` is the `Arc<dyn ContainerRuntime>` given to the container actor and `store` the host's
`Arc<dyn OutputStore>`. A host without `cicada-app` spawns `archiver.run(stop)` with Tokio itself.) For a block that
runs, the doc example of `LogArchiver` (`crates/cicada-containers/src/archive/mod.rs`) uses `FakeRuntime` and
`MemoryOutputStore` with a stop after one second.

How it works, in short:

- **Which containers.** The container actor puts the label `cicada.container.prefix` (`PREFIX_LABEL`) with its prefix
  on every container it creates, after the job's `spec`, so a job cannot drop it. The archiver lists the containers of
  that label; give it the same prefix as the job's `ContainerSettings`.
- **Rounds that never wait.** Every `interval` (2 s) a round lists the containers and starts a read for each one that
  is not drained and not still being read, up to `max_parallel` (16) at once, and does not wait for them: a slow log
  holds up only itself. A read appends each page of `logs_since` (`page_lines` lines, 500) from the stored cursor
  with the guarded `append_logs`. A running container is read again each round; an ended one is drained once, then
  skipped.
- **Once and in order.** Every line is stored once, in order, also after a restart (the new archiver reads on from
  the stored cursor) and with two archivers on one store (the second writer of a page gets `Conflict` and reads the
  cursor again).
- **Errors never end it.** Only `stop` ends `run`. A log that is `Unsupported` (such as Docker's log driver `none`)
  is logged once and not read again; a container that is gone is dropped; a store that refuses the name (`Invalid`)
  skips the container; every other error, and a failed `list`, is tried again in a later round, and from the fifth
  failure in a row the wait doubles, up to 60 s. Errors are logged through `tracing`.
- **One per Docker daemon.** Each archiver repeats every log scan of its daemon, and Docker scans a log from the
  cursor's second on each read: run one archiver per daemon, not one per node. Two stay correct, but cost twice.

**Caveats.**

- A container made before 0.22.1 has no label and is not archived, also when the actor adopts it.
- Lines a container writes after the last read are lost when the container is removed before the next round. A later
  change (`container-output-collect`) drains the rest at the end of a container.
- Do not remove a container's outputs (`OutputStore::remove`) while the container exists: the cursor would be `None`
  again and the archiver would store the log from the start once more.
- `ArchiveSettings::interval` and `max_parallel` must be positive.

## 2. The label in the actor's section

**Where.** `docs/guide/containers.md`, `Rules for hosts` (or `Define a job`), one sentence.
**Text.** The actor adds the label `cicada.container.prefix` with its prefix to every spec the job builds, after
`spec`; a job's own labels stay, and a job that sets this label is overruled. The log archiver finds containers by it.
**Caveats.** None.

## 3. The dependency change

**Where.** `docs/how-to/upgrading.md`, the 0.22.1 notes, if the docs session keeps a section per patch; else none.
**Text.** `cicada-containers` now depends on `tokio` without a feature and on `tracing`; both were in the workspace
already. Nothing to change for users.
**Caveats.** None.

## Blocking

None.

## Status

Ran: `cargo test -p cicada-containers --all-features` (197 unit, 46 actor, 14 doc tests pass),
`cargo test -p cicada-examples --test docs` (11 pass) and `cargo test -p cicada-examples --doc` (41 pass). The text above is
proposed; no prose is written yet.
