# Docs handoff for container-output-retention

Version: 0.23.0 (BREAKING; in the unreleased 0.23.0 section opened by container-progress). Refresh the dependency pins
of `docs/how-to/install.md` once it is published.

## What changed

With a store and a retention, the container actor deletes a container's stored log and files a set time after the
container's removal. New and changed public items of `cicada-containers`:

- `ContainerSettings::output_retention: Option<Duration>`, default `None` (kept until the host deletes them).
- `ContainerState` gains `outputs_until: Option<Timestamp>` (wire `outputsUntil`): set only in the phase `removed`,
  cleared once the outputs are deleted.
- The rustdoc of `ContainerActor::with_store`, `ContainerMsg::Remove` and `ContainerMsg::Check` says what the retention
  does and that a deleted actor leaves its outputs.

Proven by the retention tests in `crates/cicada-containers/tests/outputs.rs`
(`stored_outputs_are_deleted_a_retention_after_the_removal_and_not_before`,
`without_a_retention_or_a_store_nothing_is_deleted`, `a_zero_retention_deletes_at_once_and_a_huge_one_is_capped`,
`a_store_down_at_the_due_time_is_tried_again_with_the_backoff_and_then_deletes`,
`a_store_that_refuses_the_name_clears_the_due_time_without_retrying`,
`a_lost_retention_poll_comes_back_on_check_and_deletes_at_the_saved_time`,
`a_restart_with_the_retention_unset_still_deletes_at_the_saved_time`), the unit tests in `src/actor/wire.rs`
(`an_inconsistent_state_is_refused`: the 0.22 state without the field, and the field outside `removed`) and the test
job's contract snapshot `tests/contracts/test.container.json`.

## 1. Retention of stored outputs

**Where.** `docs/guide/containers.md`, after `### Outputs at the end` (container-output-collect's handoff), a new
section such as `### Retention`.
**Text.** A host keeps a container's stored log and files for a while after the container is gone, then lets the actor
delete them:

```rust
let mut settings = ContainerSettings::new("ci-")?;
settings.output_retention = Some(Duration::from_secs(7 * 24 * 3600)); // keep a week after the removal
let actor = ContainerActor::new(Tests, runtime, settings).with_store(store);
```

(the design's example; the setting is compiled in `crates/cicada-containers/tests/outputs.rs`, `retained`. Not a doc
example yet: the guide block would be the first to compile it.)

- **When.** The clock starts at the container's removal (`Remove`), not at its end: a container its owner keeps keeps
  its outputs. The removal saves the due time in the state (`outputsUntil`), and a poll then calls
  `OutputStore::remove` for the container's name. Zero deletes in the poll right after the removal. The container is
  removed from the runtime first, so a `LogArchiver` does not store its log again.
- **The saved time decides.** A retention changed or unset after the removal does not move it; a restart keeps it.
- **Errors.** A store that is `Unavailable` or fails with `Other` is tried again with the backoff, up to `max_backoff`,
  without limit: the deletion is never given up silently. A name the store refuses (`Invalid`), and a node without a
  store at the due time, clear the due time with a warning in the log; the outputs then stay.
- **A lost poll.** `Check` re-arms it at the saved time, or at once when that passed.
- **What stays.** The actor's own record (its state with the end) stays after the deletion; Cicada does not delete
  actor states.

**Caveats.** An actor deleted before its due time leaves its outputs in the store: call `OutputStore::remove` with the
container's name (`prefix` plus key) when deleting it. The actor does not list stored outputs whose actor is gone; a host
lists its own store. A container removed before 0.23.0, or by an actor without a store or a retention, gets no due
time, and its outputs stay until the host deletes them. The name is computed from the settings' prefix at the due time:
do not change the prefix while outputs wait.

## 2. Upgrade note

**Where.** `docs/how-to/upgrading.md`, the 0.23.0 section.
**Text.** The state of a container job gains `outputsUntil` (part of the same contract change as `progressType` and
`pending`). Code that builds `ContainerState` by its fields adds `outputs_until: None`. Containers removed before the
upgrade get no due time: delete their stored outputs yourself, or keep them.

## Blocking

None.

## Status

`cargo test -p cicada-containers --all-features` passed; the finishing checks are listed in the review request. All
prose above is proposed text; none of it is written in the protected docs yet.
