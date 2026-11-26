Verdict: not yet

## Blocking

1. **The age rule cannot be built, and it guards nothing.** design.md Decision 2 and the spec's requirement say "older than one interval", but `ContainerSummary` (runtime.rs) has only `name` and `status`: no creation time. The "slow create" scenario also cannot happen. A send needs an existing actor (architecture.md, commit-rejection row), so the container actor is committed before its step calls `create`. A crash between `create` and commit leaves the actor in `New`, and the next step adopts the container. My fix: drop the age rule and the slow-create scenario, and report on the first round with no actor. The other option is "missing in two rounds in a row", kept in memory. Do not add a time field to `ContainerSummary`: that is a runtime API change.
2. **The error path has no scenario.** The spec says a failed `list` or `get_actor` is retried, but no scenario shows it. The same goes for "an actor that shows up later is dropped" and "a removed container leaves the set". Add scenarios for at least the failed list and the actor that appears.

## Notes

1. A labelled container whose name does not start with the prefix (made by hand) has no key. Say what happens: skip it and log it, or report it.
2. `new` refuses a zero interval, so it returns `Result`, as `LogArchiver::new` does. The example lacks `?`, and `runtime.clone()` must be an `Arc<dyn ContainerRuntime>`.
3. Two stores on one daemon with the same prefix (say staging and prod) see each other's containers as orphans. Name this limit beside "once per daemon". Also name that containers made before 0.22.1 carry no label and are never seen.
4. `on_orphan` is a sync `Fn(&str)`. To message an actor, the host must spawn a task or use a channel. The guide handoff should say that.

## Questions for the owner

None.

## Not checked

`App::task` lifetimes in the example, beyond its signature (`FnOnce(ActorSystem, Stopping)`). How the archiver's tests set up `FakeRuntime` and `TestActors` together.
