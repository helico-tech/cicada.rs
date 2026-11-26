# Docs handoff for placement-offers

Version: 0.24.0 (unreleased; the placement line ships as one minor version once `placement-drain-and-gone` has
landed). Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-actors` gains the placement pool, `cicada.placement/<pool>` (ADR 0067): `Placement` (the ref,
`Placement::named(pool)`), `PlacementActor`, `PlacementMsg` (`Offer`, `Check`, `Rearm`, `Alert`, `Alerts`, with the
checked constructors `PlacementMsg::offer(node, instance, labels, places)` and `PlacementMsg::alerts(&target,
message_type)`), `PlacementState::new(silence, max_nodes)`, `PlacementNode` (`instance`, `labels`, `places`, `heardAt`,
`status`), `PlacementNodeStatus` (`live`, `silent`), `PlacementAlerts` (`to`, `messageType`), `PlacementNotice` (`pool`,
`node`, `kind`), `PlacementNoticeKind` (`silent`, `back`) and `InvalidPlacement`. Its contract is generated into
`docs/reference/contracts/cicada.placement.json`. Proved by `crates/cicada-actors/tests/placement.rs` (nine tests: a
renewed offer, silence and return, no check without a live node, a purged check and `Rearm`, a `Rearm` beside a live
check, a foreign `Check` or `Alert`, a failing alert target, a pool without a target, the bounds), the unit tests and
rustdoc examples in `crates/cicada-actors/src/placement.rs`, and the samples in `crates/cicada-actors/tests/wire_formats.rs`.

## 1. A placement pool section in the actors guide

**Where.** `docs/guide/actors.md`, a new section `Placement pool` after `Permit pool`.
**Text.** Work that must run on a particular kind of machine needs to know which nodes offer room. A placement pool,
`cicada.placement/<pool>`, keeps what each node offers: its labels (such as the processor kind, `docker`) and its number
of places, renewed at an interval. A node that stops offering is marked silent and gets no new places; the pool's alert
target hears of it once. (Asks for a place come with the next changes of the line.)

```rust
node.host(PlacementActor);
let pool = Placement::named("runners");                                              // cicada.placement/runners
system.create(&pool, &PlacementState::new(Duration::from_secs(60), 1000)?).await?;  // silence, max nodes
system.send(&pool, PlacementMsg::alerts(&Ops, "Placement")?).await?;               // where notices go
// A member, every 15 s while its runtime answers (instance: random per process start):
system.send(&pool, PlacementMsg::offer("factory-mac-1", &instance, ["arm64", "docker"], 4)?).await?;
// Ops gets Placement(PlacementNotice { pool: "runners", node: "factory-mac-1", kind: Silent }) once.
```

(From the design; the compiled samples are the rustdoc examples of `PlacementMsg::offer` and `PlacementState::new` and
the test `an_offer_replaces_the_last_one_and_records_when_the_node_was_heard` in
`crates/cicada-actors/tests/placement.rs`.) Points to cover:

- **Creating a pool.** The host creates each pool with `PlacementState::new(silence, max_nodes)`: silence 1 second
  through 1 day, 1 through 1,000 nodes. The settings are fixed at the create; a create of a pool that exists changes
  nothing; a message to a pool that does not exist fails as any send does.
- **Offers.** `PlacementMsg::offer(node, instance, labels, places)`: `node` is the id the member's node pins its actors
  under (1 through 200 characters, not blank, no NUL); `instance` an id the member draws at each process start (same
  bounds), so two processes offering under one node id can be told apart later; up to 32 labels of 1 through 100
  characters, sorted and kept once; 0 through 1,000 places (`0` keeps the node known but full). Each offer replaces the
  node's entry and records the store time of its turn as `heardAt`. Send one at an interval well below the silence
  (for example every 15 s for a silence of 60 s).
- **Silence.** The pool arms one `Check` to itself at the earliest end of a live node's silence (`heardAt` plus the
  silence), and keeps its time in `armed`. At the check each node past it becomes `silent`. A silent node's next offer
  makes it `live` again.
- **Alerts.** `PlacementMsg::alerts(&target, message_type)` sets where notices go; the last one stands, and without one
  no notices are sent. The target's family takes a variant such as `Placement(PlacementNotice)`. Kinds: `silent` once per
  silence, `back` when a silent node offers again. Each notice is sent in a turn of its own (`Alert`), so a target that is
  gone or does not take the message fails only that alert: the node is still silent.
- **A lost check.** A purged or failed `Check` leaves the pool without a timer; anyone may send `Rearm`, which arms it at
  the old time, at once when that passed. A `Check` or `Alert` from anyone but the pool is refused for good.
- **Reading it.** The pool answers no query: read its state like any actor's (`ActorSystem::get_actor`, the operator
  commands, the terminal UI); its contract is `docs/reference/contracts/cicada.placement.json`.
- **Bounds.** A message out of bounds, and an offer from a new node when the pool knows `maxNodes` nodes, is refused for
  good with the state unchanged; a known node can always renew.

**Caveats.**
- Do not host the pool only on its member nodes: when they all go quiet, nobody runs the check, so nobody is told.
- Keep pools small and intervals gentle. At the upper bounds (1,000 nodes, 32 long labels each) the state is about 3 MB,
  rewritten at each offer; 1,000 nodes every 15 s is about 67 turns a second on one actor.
- Silence is counted from the store time an offer was handled, not from when the member sent it; a paused or slow pool
  marks a node late, never early.
- The pool's contract grows in the next changes of the line (asks, holders, drain and gone); they ship together as one
  version.

## 2. The ready-made actors list in the architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crates table.
**Text.** Add: a placement pool (`cicada.placement`, ADR 0067) to which nodes offer instance, labels and places, renewed;
its own self-only `Check`, armed at the earliest end of a live node's silence (re-armed by `Rearm`), marks a quiet node
silent, and each change (`silent` once per silence, `back` at the next offer) is sent to the pool's alert target in an
`Alert` turn of its own.

## 3. The map in AGENTS.md

**Where.** `AGENTS.md`, the `crates/cicada-actors` row of `Map`.
**Text.** Add `placement` to the list of modules (between `permits` and `job_graph`).

## 4. The consumer skill's route (after section 1 exists)

**Where.** `skills/cicada/SKILL.md`, the `Ready-made actors` route.
**Text.** Once `docs/guide/actors.md#placement-pool` exists, add it and `docs/reference/contracts/cicada.placement.json`
to the route. The skill belongs to the change loop; it is not changed here because the heading does not exist yet.

## Blocking

None.

## Status

The text above is proposed; no protected prose is written. Checks run are listed in the build report.
