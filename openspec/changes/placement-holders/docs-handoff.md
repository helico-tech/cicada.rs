# Docs handoff for placement-holders

Version: 0.24.0 (unreleased; the placement line ships as one minor version once `placement-drain-and-gone` has
landed). Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

A deleted holder frees its place in the placement pool `cicada.placement/<pool>`. The pool watches the target of each
holder and waiting ask (the actor named by the ask's `to`) with `Turn::watch_existing`, from the commit that first
records the ask, granted or queued, and unwatches it at the end of the turn after which it holds and waits for nothing.
When that actor is deleted, before or after the commit, the pool gets one `Stopped`: every place held for it is freed,
once, and its waiting asks leave the line; freed places go to the line as after a `Release`. An ask whose target is the
pool itself is now refused for good. No new public item, message or state field: the doc comments of `PlacementMsg::Ask`,
`Failed`, `Stopped` and of `PlacementActor` say the new behaviour, and the contract reference
`docs/reference/contracts/cicada.placement.json` is regenerated (the `Ask` description). Proved by
`crates/cicada-actors/tests/placement.rs`: `the_pool_watches_the_targets_that_hold_or_wait_and_unwatches_after_the_last_release`,
`a_release_and_a_line_grant_to_one_target_leave_it_watched`, `a_holder_deleted_after_its_grant_frees_the_place_for_the_line`,
`a_target_deleted_before_its_grant_gets_no_place`, `a_deleted_target_with_two_holders_frees_both_places`,
`a_failed_message_of_a_holder_changes_nothing`,
`a_stopped_for_the_pool_a_client_or_a_target_that_holds_nothing_changes_nothing` and
`an_ask_to_the_pool_itself_is_refused`; `a_grant_from_the_line_to_a_gone_target_is_freed` now deletes the target after
the grant's commit.

## 1. A deleted holder in the guide's placement pool section

**Where.** `docs/guide/actors.md`, `Placement pool`, the `**Releasing.**` paragraph: replace "A holder that is gone
without a `Release` keeps its place: release in every path that ends the work."
**Text.** A holder that is deleted frees its place: the pool watches the target of each holder and waiting ask while it
holds or waits, so when that actor goes away, also before the pool recorded its ask, every place held for it is freed
and its waiting asks leave the line, once. A holder actor need not release before it stops; a holder that ends its work
and lives on still releases. Places are freed per target: two holders whose answers go to one actor are both freed when
it goes.
**Caveats.**
- Only the target's delete frees its places: a target that lives on but has stopped its work keeps them until it
  releases.
- The watch also tells the pool of every failed message of a target: each costs the pool a turn that changes nothing.
  A target with many failing messages adds that load.
- An ask whose target is the pool itself is refused for good (the pool's watch on itself frees undelivered grants).
- The pool keeps at most one watch per distinct target, so at most `maxHolders` + `maxWaiting` watches.

## 2. The ready-made actors list in the architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crates table, the placement pool entry.
**Text.** Add: the pool watches each holder's and waiting ask's target with `watch_existing`, so a deleted target frees
its places and leaves the line, once.

## Blocking

None.

## Status

Proposed text only; nothing protected was edited. Checks: see the report of this build.
