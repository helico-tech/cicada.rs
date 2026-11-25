# Docs handoff for watch-absent-target

Version: 0.22.0 (BREAKING; the unreleased batch). Refresh the dependency pins of `docs/how-to/install.md` once it is
published.

## What changed

`Turn::watch_existing(target)` (ADR 0063) watches one actor that must exist. The commit checks the target where it
applies the watches, in the order of the calls and before the turn's deletes and creates:

- there: a plain watch, exactly as `Turn::watch` sets it (it survives a delete and re-create, ADR 0023);
- not there: one `Stopped { actor, cause: deleted, by: null }` in the watcher's mailbox in the same transaction, with
  the turn's message as `causedBy`, the absent actor as sender, numbered with the commit's other notifications
  (`<event>.N`); the pair holds no watch afterwards, an older stored one included; a later create of that address is
  not watched.

A delete in the same turn finds the watch set and writes its own `Stopped` (with `by` the turn's actor). An actor
created in the same turn is not there yet: a `Stopped`, so such a turn uses `watch`. The watcher itself is always
there. Two calls for one absent actor give one `Stopped`; `watch_existing` then `unwatch` leaves no watch; `watch` then
an absent `watch_existing` gives a `Stopped` and no watch. A watcher that stops in the same turn gets nothing. A type
prefix or `*` is `TurnError::Invalid` at the call (the turn goes on), as is a family without `Failed` and `Stopped`.
A delete by another node is seen exactly once: on PostgreSQL the commit locks the target's row with its other rows.

Wire: the remote turn report's watch change (`WatchChangeView`) carries `"existing": true`, left out when false; a body
without it reads as a plain watch change. `existing` with a type target, `*` or `set: false` is an invalid report
(`400`). `docs/openapi.json` regenerated; `fixtures/leases.json` gains `complete-request-returned-watch-existing`
(valid), `complete-request-watch-existing-false` (accepted) and `complete-request-watch-existing-on-a-type`,
`complete-request-watch-existing-without-set` (invalid, stage conversion).

Breaking: `WatchChange` and `WatchChangeView` gain the public field `existing: bool`; struct literals and exhaustive
patterns add `existing: false`. No schema migration, no contract hash change.

Tests and examples that prove it:

- `crates/cicada/src/tests/conformance/watch_existing.rs` (every backend): `watch_existing_of_a_present_actor_sets_the_watch_and_a_later_delete_writes_one_stopped`,
  `watch_existing_of_an_absent_actor_writes_one_stopped_and_leaves_no_watch`,
  `watch_existing_of_a_type_or_without_set_is_invalid`,
  `watch_existing_with_a_delete_of_the_target_in_the_same_turn_writes_the_stopped_of_the_delete`,
  `watch_existing_of_an_actor_created_in_the_same_turn_writes_a_stopped`,
  `watch_existing_of_the_watcher_itself_sets_the_watch`,
  `watch_existing_applies_in_the_order_of_the_calls_and_two_calls_give_one_stopped`,
  `watch_existing_numbers_its_stopped_with_the_other_notifications_of_the_event`,
  `watch_existing_racing_a_delete_of_the_target_gives_exactly_one_stopped`.
- `crates/cicada/tests/watch_existing.rs` (public API, virtual time): the pool case
  `a_run_deleted_before_the_pools_turn_frees_its_place_through_watch_existing`, its contrast
  `a_plain_watch_of_a_run_that_is_gone_keeps_the_place`, `a_run_that_is_there_is_watched_and_its_stop_frees_the_place`
  and `watch_existing_of_a_type_or_every_type_fails_at_the_call_and_the_turn_goes_on`.
- `crates/cicada-client/tests/remote_worker.rs::a_watch_existing_of_an_absent_actor_over_http_writes_the_stopped_of_a_local_commit`.
- `crates/cicada/src/api/leases/tests.rs::a_watch_change_carries_existing_when_true_and_an_invalid_one_fails_conversion`
  and the two refused reports in `crates/cicada-axum/tests/leases.rs::malformed_bodies_are_400_before_any_operation_and_old_routes_keep_their_parsing`.

## 1. The core guide

**Where.** `docs/guide/core.md`, `Watches`.
**Text.** After "Use `turn.watch(target)` / `unwatch(target)` …", add: when a turn must learn that an actor ends and is
not sure it is still there, use `turn.watch_existing(&actor)`. If the actor is there at the commit, it is a plain
watch; if it is gone, the watcher gets a `Stopped` with the cause `deleted` and `by` null at once, and no watch is
kept. So a pool that gives a place to a run and watches it frees the place even when the run went before the pool's
turn; a plain watch would land on nothing and never tell. One actor only: a type prefix or `*` is refused at the call.
A Rust block, if one is wanted, can follow `PoolActor::handle` in `crates/cicada/tests/watch_existing.rs`
(`PoolMsg::Track { run, plain: false } => turn.watch_existing(&Run { id: run })?`); no doctest exists for it yet.
**Caveats.** An actor created in the same turn is not there yet at the check and gives a `Stopped`: use `watch` for
it. `deleted` with `by` null now also means "not there when the watch was set"; the watcher cannot tell that from a
delete from outside. A later create of the address is not watched.

## 2. Architecture: Watch and Notification

**Where.** `docs/architecture.md`, `Domain language`, the rows `Watch` and `Notification`; and `Turn semantics`, the
paragraph that starts "A watch is set and removed by the turn of its watcher".
**Text.** Watch: "… it need not exist, and the watch survives its delete and re-create. A turn sets and removes the
watches of its own actor (`turn.watch`, `turn.unwatch`), or watches an actor that must exist (`turn.watch_existing`,
ADR 0063): the commit sets the watch when the actor is there and otherwise writes its `Stopped` and keeps no watch;
…". Notification: add that a `Stopped` is also written by the commit of a `watch_existing` whose actor is not there
(`deleted`, `by` null, `causedBy` the turn's message). Turn semantics: the commit checks each `watch_existing` target
where it applies the watches, in the order of the calls, before its deletes and creates; on PostgreSQL the target's row
is in the commit's ordered `FOR UPDATE` set, so a concurrent delete is seen either before the check or after the watch.
**Caveats.** `turn.watch` is unchanged; "the target need not exist" still holds for it.

## 3. Guarantees

**Where.** `docs/guarantees.md`, rows `G2 Atomic` and `G4 Effects once`; and the G2 row of `docs/architecture.md`,
`Guarantees`.
**Text.** G2: name the `Stopped` of an absent `watch_existing` target among "the notifications of the event" (it
commits with the turn or not at all); add `crates/cicada/src/tests/conformance/watch_existing.rs::watch_existing_of_an_absent_actor_writes_one_stopped_and_leaves_no_watch`
to its tests. G4: add `crates/cicada/src/tests/conformance/watch_existing.rs::watch_existing_racing_a_delete_of_the_target_gives_exactly_one_stopped`
(a delete racing the commit gives exactly one `Stopped`, on every backend).
**Caveats.** None.

## 4. Upgrading to 0.22.0

**Where.** `docs/how-to/upgrading.md`, a new section `Upgrade to 0.22.0` above `Update dependencies for 0.20.6`.
**Text.** Upgrade every HTTP server before a worker runs a turn that calls `watch_existing`: a 0.21 server refuses the
unknown field `existing` and the report with it. A 0.21 worker keeps working against a 0.22 server. Rust code that
builds `WatchChange` or `WatchChangeView` field by field adds `existing: false`; exhaustive patterns bind or ignore it.
No store migration, no contract hash change, no doc example builds these structs.
**Caveats.** Direct-store nodes of 0.21 and 0.22 can share a store; only the HTTP report changed.

## 5. HTTP guide (optional)

**Where.** `docs/guide/http.md`, where the remote turn report's effects are described, if they are.
**Text.** A watch change may carry `"existing": true` (from `watch_existing`), only with `set: true` and an actor
target; otherwise the report is `400`.
**Caveats.** Left out when false, so older bodies are unchanged.

## Blocking

None. No doctest, link, guarantee row or skill route fails: the docs tests, the doc examples and the skill tests pass
on this branch without prose changes.

## Status

Ran on this branch: `cargo test -p cicada-examples --test docs`, `--doc` and `--test skill` (pass, see the build
report). All text above is proposed; nothing in `README.md`, `AGENTS.md` or the handwritten docs is edited. Not
written yet: every section above.
