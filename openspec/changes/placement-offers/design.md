# Design

The first change of the placement-pool line (owner: PL-SPLIT). Nodes offer places to a pool, and later changes let work
ask for one and get a node to pin a child to (ADR 0052). This change is the offers and their silence: who has what,
and who has gone quiet. Factory's pool (`pool.rs`, `member.rs`) and its git mirror per machine need it.

```rust
node.host(PlacementActor::new(PlacementSettings::new()));          // silence 60 s, at most 1000 nodes
let pool = Placement::named("runners");                              // cicada.placement/runners
// A member, every 15 s while its runtime answers:
system.send(&pool, PlacementMsg::offer("factory-mac-1", ["arm64", "docker"], 4)?).await?;
// The pool's alert target gets {"pool": "runners", "node": "factory-mac-1", "kind": "silent"} once.
```

## Decisions

1. **A sibling of the permit pool, not a wrapper.** A new ready-made actor `cicada.placement/<pool>` in
   `cicada-actors`, with the permit pool's patterns: checked messages through typed constructors, a state decoded
   through checks, bounded lists, notices sent with `send_raw` to a target the sender names. The permit pool's logic is
   private and counts one capacity, not places per node with labels, so it is not reused. ADR 0067 records the pool.
   Not taken: the node table (rows are for display, shared ids share a row, HTTP workers have none: ADR 0054 warns
   against placement from it); the heartbeat watch (one watched thing per actor, and its alarm is its only output).
2. **An offer is a renewal.** `Offer { node, labels, places }`: `node` a `NodeId`-shaped string (the id the member's
   node pins under), `labels` a set of up to 32 strings (1 to 100 characters, no NUL), `places` 0 to 1000. The pool
   keeps per node its labels, places and `heardAt` (the store time of the turn), replacing the last offer. A member
   sends it at an interval while it can run work; `places: 0` keeps the node known but full. Up to
   `max_nodes` (default 1000) nodes; an offer from a new node past that is refused and logged.
3. **Silence by the pool's own timer.** The pool arms one check at the earliest `heardAt + silence` (default 60 s,
   `PlacementSettings`); at the check, each node past it becomes `silent`: it gets no new places (the asks change uses
   that), and the pool sends one `PlacementNotice { pool, node, kind: silent }` to its alert target, once per silence.
   An offer from a silent node makes it `live` again and sends `kind: back`. The alert target is set with
   `Alerts { to, messageType }` (the last one stands; none: no notices). Not taken: an alert per check (noise).
4. **Read and forget.** `Nodes` gives nothing back by itself (actors answer by message): the pool's state is read like
   any actor's (`ActorSystem::get_actor`, the operator commands, the terminal UI), and its schema is the contract.
   `Forget { node }` removes a silent node from the state (refused for a live one), for an operator who retired a
   machine; draining and "gone" with holders come in `placement-drain-and-gone`.

## Tests

On virtual time with `TestActors`: offers renew and replace; a node not heard from for the silence becomes silent and
one notice goes out, and none on the next checks; an offer brings it back with `back`; `Forget` of a silent node
removes it and is refused for a live one; the bounds (labels, places, nodes) are refused with a reason; the state
decodes only when valid; the contract and samples in `tests/wire_formats.rs` and `docs/reference/contracts`.

## Docs

The handoff gives the actors guide a placement pool section (offers, silence, alerts), and the contracts reference is
generated. Nothing breaks.
