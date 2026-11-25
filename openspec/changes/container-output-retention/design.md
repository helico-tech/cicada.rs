# Design

The fifth change of the container-outputs line. A host keeps a container's stored log and files for a time after the
container is gone, then deletes them. Factory does this itself today.

```rust
let mut settings = ContainerSettings::new("ci-")?;
settings.output_retention = Some(Duration::from_secs(7 * 24 * 3600));   // keep a week after removal
let actor = ContainerActor::new(Tests, runtime, settings).with_store(store);
```

## Decisions

1. **The container actor deletes, on its own timer, after the removal.** `ContainerSettings::output_retention:
   Option<Duration>` (default `None`: keep forever, as today). When the actor has a store and a retention, the step
   that removes the container (phase `Removed`) also saves `outputs_until` = now + retention in the state and arms a
   poll then; the poll calls `OutputStore::remove(name)` and clears `outputs_until`. The container is removed from the
   runtime first, so no `LogArchiver` follows it any more (`container-log-archive`), and a re-archive from the start
   cannot happen. Not taken: the scheduler actor (one more actor and protocol for a timer the container actor has);
   a retention counted from the end (a container kept by its owner would lose its outputs while it still exists).
2. **A state field: part of the 0.23.0 contract change.** `outputs_until: Option<Timestamp>` joins the state, so it
   ships in the breaking batch with `container-progress` and `container-output-collect`: hosts change their container
   job contracts once. A state from 0.22 has none and is not retained.
3. **Work left while outputs wait; errors keep trying.** Today the phase `Removed` has no work left, so a `Check`
   changes
   nothing there. With `outputs_until` set, `work_left` is true in `Removed`, so a lost due poll comes back through
   `Check` (the rule "A lost poll comes back" is modified to say so). `OutputError::Unavailable` and `Other` keep
   trying as a removal does today: each failure arms the next try with the backoff, up to `max_backoff`, with no
   limit, so the deletion is never given up silently. `Invalid` clears `outputs_until` with a warning (a name the
   store refuses holds nothing). Removing nothing is `Ok`. The saved `outputs_until` decides: a retention changed or
   unset after the removal does not move it; an actor without a store at the due time clears it with a warning.
   `outputs_until` is valid only in `Removed`, is capped at the largest instant, and a retention of zero deletes in
   the poll right after the removal. The actor's own record stays after the deletion.
4. **A deleted actor leaves its outputs.** When a host deletes the container actor before its retention passes, its
   outputs stay in the store; the rustdoc says to call `OutputStore::remove` then, and the outputs of actors that are
   gone are not listed by this change (a host can list its own store).

## Tests

With `FakeRuntime`, `MemoryOutputStore` and `TestActors` on virtual time: a removed container's outputs are deleted
after
the retention, not before; no retention or no store deletes nothing; a store `Unavailable` at the due time keeps
retrying with the backoff and then deletes; a lost due poll and a `Check` still delete; a restart between removal and
the due time still deletes; `Invalid` clears the due time; a retention unset after the removal still deletes at the
saved time; `outputs_until` is cleared after; a 0.22 state decodes without it; a contract snapshot.

## Docs

The handoff asks for the guide's retention paragraph (the setting, after removal, a deleted actor leaves outputs) and a
line in the 0.23.0 upgrade section.
