# Design

The first change of the placement-pool line (owner: PL-SPLIT). Nodes offer places to a pool, and later changes let work
ask for one and get a node to pin a child to (ADR 0052). This change is the offers and their silence: who has what,
and who has gone quiet. Factory's pool (`pool.rs`, `member.rs`) and its git mirror per machine need it.

```rust
node.host(PlacementActor);
let pool = Placement::named("runners");                                              // cicada.placement/runners
system.create(&pool, PlacementState::new(Duration::from_secs(60), 1000)?).await?;  // silence, max nodes
// A member, every 15 s while its runtime answers (instance: random per process start):
system.send(&pool, PlacementMsg::offer("factory-mac-1", &instance, ["arm64", "docker"], 4)?).await?;
// The pool's alert target gets {"pool": "runners", "node": "factory-mac-1", "kind": "silent"} once.
```

## Decisions

1. **A sibling of the permit pool, not a wrapper.** A new ready-made actor `cicada.placement/<pool>` in
   `cicada-actors`, with the permit pool's patterns: checked messages through typed constructors, a state decoded
   through checks, bounded lists, notices sent with `send_raw` to a target the sender names. The permit pool's logic is
   private and counts one capacity, not places per node with labels, so it is not reused. ADR 0067 records the pool.
   Not taken: the node table (rows are for display, shared ids share a row, HTTP workers have none: ADR 0054 warns
   against placement from it); the heartbeat watch (one watched thing per actor, and its alarm is its only output).
2. **Settings in the state, fixed at the create.** As the permit pool and the heartbeat watch do, the pool's settings
   live in its state: `PlacementState::new(silence, max_nodes)` (silence 1 s to 1 day, max nodes 1 to 1000). The host
   creates each pool explicitly with that state (`system.create`), like a permit pool; a message to a pool that does
   not exist fails as any send does.
3. **An offer is a renewal.** `Offer { node, instance, labels, places }`: `node` the id the member's node pins under
   (1 to 200 characters, no NUL, as `NodeId`), `instance` an id the member draws at each process start (same bounds;
   `placement-member` uses it to see two machines offering under one node id), `labels` up to 32 strings (1 to 100
   characters, no NUL; duplicates removed, kept sorted), `places` 0 to 1000. The pool keeps per node its instance,
   labels, places and `heardAt` (the store time of the turn), replacing the last offer. A member sends it at an
   interval shorter than the silence; `places: 0` keeps the node known but full. An offer from a new node past
   `max_nodes`, and any out-of-bounds message, is refused for good (a permanent error, as in the permit pool). At the
   upper bounds the state is about 3 MB, rewritten at each offer: the handoff says to keep pools small and intervals
   gentle (1000 nodes every 15 s is about 67 turns a second on one actor).
4. **Silence by the pool's own timer, robust to loss.** The pool arms one `Check` (a message it sends itself; a `Check`
   from anyone else is refused) at the earliest `heardAt + silence`, and keeps the time in `armed`. At the check, each
   node past it becomes `silent`: it gets no new places (the asks change uses that). A lost or purged check comes back
   through `Rearm` (anyone may send it; it re-arms at `armed`, or at once when that passed), as in the heartbeat watch.
   **Notices in turns of their own:** the check queues one `Alert { node, kind }` to itself per changed node, and each
   `Alert` turn sends `PlacementNotice { pool, node, kind }` to the alert target, so a failing target fails only that
   alert, never the check. Kinds: `silent` once per silence, `back` when a silent node offers again. The target is set
   with `Alerts { to, messageType }` (the last one stands; none: no notices). Not taken: an alert per check (noise).
5. **Read, not asked.** The pool answers no query: its state is read like any actor's (`ActorSystem::get_actor`, the
   operator commands, the terminal UI), and its schema is the contract. Removing a node comes with
   `placement-drain-and-gone` (`Gone`, which also tells holders), not here.
6. **One release for the line.** The pool's contract grows with `placement-asks`, `placement-holders` and
   `placement-drain-and-gone`; the line is released as one minor version once those have landed, so no host sees a
   pool contract that changes the next release.

## Tests

On virtual time with `TestActors`: offers renew and replace, labels deduplicated and sorted; a node not heard from for
the silence becomes silent and one notice goes out, and none on the next checks; an offer brings it back with `back`;
a `Check` from another actor is refused; a purged check comes back through `Rearm`; a failing alert target fails only
its `Alert`, and the node is still silent; the bounds (node id, instance, labels, places, nodes) are refused for good;
the state decodes only when valid; the contract and samples in `tests/wire_formats.rs` and `docs/reference/contracts`.

## Docs

The handoff gives the actors guide a placement pool section (creating a pool, offers, silence, alerts, sizes), and
warns against hosting the pool only on its member nodes (when they all go quiet, nobody runs the check). The contracts
reference is generated. Nothing breaks.
