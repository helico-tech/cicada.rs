# Docs handoff for placement-asks

Version: 0.24.0 (unreleased; the placement line ships as one minor version once `placement-drain-and-gone` has
landed). Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The placement pool, `cicada.placement/<pool>`, now grants places (ADR 0068). New public items in `cicada-actors`:
`PlacementMsg::Ask`, `Release`, `Cancel`, `Notify`, `Failed`, `Stopped`, with the checked constructors
`PlacementMsg::ask(holder, needs, &target, message_type)`, `PlacementMsg::release(holder, grant)` and
`PlacementMsg::cancel(holder)`; `PlacementAnswer` (`pool`, `holder`, `kind`, `grant?`, `node?`, `reason?`),
`PlacementAnswerKind` (`granted`, `queued`, `refused`), `PlacementRefusal` (`noNode`, `lineFull`, `poolFull`),
`PlacementHolder` and `PlacementAsk`. `PlacementState::new` now takes `(silence, max_nodes, max_waiting, max_holders)`,
and the state gains `maxWaiting`, `maxHolders`, `nextGrant`, `holders` and `waiting`. The contract reference
`docs/reference/contracts/cicada.placement.json` is regenerated. Proved by `crates/cicada-actors/tests/placement.rs`
(fourteen new tests, each named after its scenario), the unit tests and rustdoc examples in
`crates/cicada-actors/src/placement.rs` (`PlacementMsg::ask`, `PlacementState::new`), and the samples in
`crates/cicada-actors/tests/wire_formats.rs`.

## 1. Asks in the guide's placement pool section

**Where.** `docs/guide/actors.md`, `Placement pool`: replace "(Asks for a place come with the next changes of the line.)"
and extend the example and the points.
**Text.** Work asks the pool for a place with its needs (labels a node must have), and gets the node to pin its actors
to (ADR 0052). It releases the place with its grant number when done.

```rust
system.create(&pool, &PlacementState::new(Duration::from_secs(60), 1000, 100, 1000)?).await?; // silence, max nodes, max waiting, max holders
system.send(&pool, PlacementMsg::ask("run-17", ["arm64", "docker"], &run_ref, "Placed")?).await?;
// run-17 gets Placed(PlacementAnswer { pool: "runners", holder: "run-17", kind: Granted, grant: Some(4),
//     node: Some("factory-mac-1"), .. }): create its container child pinned to factory-mac-1; when done:
system.send(&pool, PlacementMsg::release("run-17", 4)?).await?;
```

(From the design; the compiled samples are the rustdoc examples of `PlacementMsg::ask` and `PlacementState::new` and
the test `a_grant_goes_to_the_node_with_the_most_free_places_then_the_smaller_id` in
`crates/cicada-actors/tests/placement.rs`.) Points to cover:

- **Creating a pool.** `PlacementState::new(silence, max_nodes, max_waiting, max_holders)`: the line holds 0 through
  1,000 waiting asks, the pool 1 through 1,000 holders. Both are fixed at the create.
- **Asking.** `PlacementMsg::ask(holder, needs, &target, message_type)`: `holder` names this attempt (1 through 200
  characters, no NUL), unique among the pool's holders and waiting asks; use a fresh one per attempt. Up to 32 needs,
  1 through 100 characters, sorted and kept once. The target's family takes a variant such as
  `Placed(PlacementAnswer)`. The answer goes out in the ask's commit, so an ask whose target is gone fails and records
  nothing.
- **Which node.** A node fits when it is live, has every need, and has a free place (its offered places minus its
  holders). The one with the most free places wins; ties go to the smaller node id.
- **Answers.** `granted` with `grant` and `node`; `queued`; `refused` with `reason`: `noNode` when no node of the pool,
  live or silent, has every need; `lineFull` when the ask must wait and the line is full; `poolFull` when a place fits
  but the pool holds `maxHolders` and the line is full. A repeated ask of a holder answers `granted` again with the same
  grant and node; of a waiting ask, `queued`. Both keep the first ask's needs and target.
- **The line.** An ask that only silent or full nodes could meet, or one that comes while the pool holds `maxHolders`,
  waits. After every message the pool grants waiting asks in line order, but skips asks it cannot place now, so an ask
  for a rare label does not hold up the others (ADR 0068). A grant from the line is told in a turn of its own; if that
  answer cannot be delivered (the target is gone), the place is freed again. A waiting ask stays until it is granted or
  cancelled, also when no node has its needs any more (a node may come back).
- **Releasing and cancelling.** `PlacementMsg::release(holder, grant)` frees the place only when holder and grant
  match, so a late release of an older grant frees nothing. `PlacementMsg::cancel(holder)` withdraws a waiting ask; for
  a holder it does nothing.
- **Fewer places.** An offer with fewer places than holders keeps every holder; new grants wait until holders release.
  A silent node keeps its holders.

**Caveats.**
- A holder that is gone without a `Release` keeps its place until `placement-holders` lands (the pool will watch its
  holders); until then, release in every path that ends the work.
- There is no time limit on a waiting ask: a host that needs a deadline sends `Cancel` itself.
- Fairness is not strict: a rare-label ask can wait while later asks pass it.
- `Notify`, like `Check` and `Alert`, is the pool's own: one from anyone else is refused for good.

## 2. The ready-made actors list in the architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crates table, the placement pool entry.
**Text.** Add: work asks with needs and gets `granted` (grant number and node: the live node with every need and the
most free places, ties by id), `queued` or `refused` (`noNode`, `lineFull`, `poolFull`) in the ask's commit; the bounded
line is served in order, skipping asks it cannot place now (ADR 0068); `Release` names its grant; a grant from the line
is told in a `Notify` turn of its own and freed when never received.

## 3. The consumer skill's route

**Where.** `skills/cicada/SKILL.md`, the `Ready-made actors` route.
**Text.** The owner asked the change loop to add `docs/guide/actors.md#placement-pool` and
`docs/reference/contracts/cicada.placement.json` to the route once the heading is on `main`; see Status for whether
this branch has it.

## Blocking

None.

## Status

See the builder's report for the checks and their results. The text above is proposed; no protected prose is written.
