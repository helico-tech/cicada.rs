# Docs handoff for container-orphan-alert

Version: 0.23.1. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` gains `OrphanWatch<J: ContainerJob>`, `OrphanSettings` (`interval`, 5 minutes) and
`InvalidOrphanWatch` (a zero interval). `OrphanWatch::new(runtime, prefix, settings, on_orphan)` and
`run(system: ActorSystem, stop)` report each container that carries the job's `PREFIX_LABEL` but whose container actor
(`J::TYPE`, keyed by the name without the prefix) is missing in two rounds in a row: a warning in the log and one call of
`on_orphan(&str)` per orphan. It never removes a container. Proved by the tests of
`crates/cicada-containers/src/orphans/tests.rs` (nine: reported once after the second round; one with its actor not;
missing in one round only not; an actor that appears drops it; a removed one leaves the set; a failed `list` and a
failed actor read do not count; names without the prefix skipped; zero interval refused; `stop` ends `run`) and the
rustdoc examples of `OrphanWatch` and `OrphanSettings`.

## 1. Watching for containers without their actor

**Where.** `docs/guide/containers.md`, after the section on the `LogArchiver` (or a new section `Containers without
their actor`).
**Text.** A container that carries a job's prefix label but has no container actor (left by a lost store, a test, an
actor deleted by hand) runs on unnoticed. `OrphanWatch` lists the job's containers every `interval` (5 minutes) and
reads the actor of each one; a container whose actor is missing in two rounds in a row is reported once: a warning in
the log, and a call of the host's `on_orphan` with its name. Run it as an app task, once per Docker daemon:

```rust
let runtime: Arc<dyn ContainerRuntime> = Arc::new(DockerRuntime::connect()?);
let watch = OrphanWatch::<Tests>::new(runtime.clone(), "ci-", OrphanSettings::new(), |name: &str| {
    tracing::warn!(container = name, "a container without its actor");   // or the host's own alert
})?;
App::new("factory").task("orphans", move |system, stopping| async move {
    watch.run(system, async move { stopping.wait().await }).await;
    Ok(())
});
```

(From the design; the compiled example is the rustdoc of `OrphanWatch` in `crates/cicada-containers/src/orphans/mod.rs`,
on `FakeRuntime` and `TestActors`. A guide block would be `no_run`, as it connects to Docker.) `on_orphan` is a plain
function: to send a message, spawn a task or push into a channel from it. The watch removes nothing: decide with
`sweep` or by hand.
**Caveats.** What it reported is kept in memory: after a restart each orphan is reported once more; an orphan whose
actor appears is dropped, and reported again only after its actor is gone again for two rounds; a removed one is
forgotten. A failed `list` or actor read is logged and does not count as a round. Two stores on one Docker daemon with
the same prefix (staging and production) see each other's containers as orphans: give each its own prefix. Containers
made before 0.22.1 carry no label and are never seen. A labelled name without the prefix is skipped with one warning.

## Blocking

None.

## Status

`cargo test -p cicada-containers --all-features` (the nine watch tests and the two doctests pass). The text above is
proposed; no protected prose is written yet.
