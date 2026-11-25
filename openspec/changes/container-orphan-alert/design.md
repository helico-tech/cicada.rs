# Design

The sixth and last change of the container-outputs line. A container that carries a job's prefix label but has no
container actor (left by a lost store, a test, a crash between a create and its commit, an actor deleted by hand) runs
on unnoticed. Factory alerts on them; this gives every host the check.

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

## Decisions

1. **A watch outside turns, like the archiver.** `OrphanWatch<J: ContainerJob>::new(runtime, prefix, settings,
   on_orphan)` and `async fn run(self, system: ActorSystem, stop: impl Future<Output = ()>)`. Every `interval`
   (default 5 minutes), it lists the containers with `PREFIX_LABEL` = `prefix` (`container-log-archive` adds the
   label), derives each actor's address (`J::TYPE` and the name without the prefix) and asks
   `ActorSystem::get_actor`; a container whose actor does not exist is an orphan. A host runs it as an `App::task`,
   once per Docker daemon. Not taken: inside the archiver (two concerns in one loop, and the archiver needs no
   `ActorSystem`); a ready-made actor (it needs the runtime, which actors get only from the host).
2. **Alert once per container, after two rounds, never remove.** A container found without its actor in two rounds in a
   row is an orphan, so a lookup that misses once is not an alert; the runtime's list gives no creation time, and a
   container actor is saved before it creates its container (a crash between them leaves the actor, whose next step
   adopts the container), so no age rule is needed. `on_orphan(&str)`, a plain function, is called once per orphan
   name while the watch runs (an in-memory set, pruned to the names `list` returns), and the watch logs a warning too;
   a host that must send a message spawns a task or uses a channel from it. It removes nothing: `sweep` and the host
   decide. A container whose actor appears is dropped from the set; one that is removed leaves it. After a restart each
   orphan is reported once more. A labelled container whose name does not start with the prefix has no key: it is
   skipped with a warning once. Not taken: removing orphans (a container another node is about to adopt would be
   killed).
3. **Errors keep it going.** A failed `list` or `get_actor` is logged and tried next round, and does not count as a
   round without the actor; nothing ends `run`. `OrphanWatch::new` returns a `Result` and refuses a zero interval, as
   the archiver does. Limits, in the rustdoc: two stores on one Docker daemon with the same prefix (staging and prod)
   see each other's containers as orphans, so give each its own prefix; containers made before 0.22.1 carry no label
   and are never seen.
4. **Additive.** No contract or state changes; it can ship in 0.23.0 or after.

## Tests

With `FakeRuntime` and `TestActors` on virtual time: a labelled container without an actor is reported once, after the
second round, over several rounds; one with its actor is not; a missing actor in one round only is not reported; an
orphan whose actor appears is dropped and not reported again; one that is removed leaves the set; a failed `list` or
`get_actor` is retried and does not count; a name without the prefix is skipped; a zero interval is refused; `stop`
ends `run`.

## Docs

The handoff gives the container guide the watch, running it as a task, and that it never removes.
