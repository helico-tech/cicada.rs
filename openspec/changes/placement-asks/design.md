# Design

The second change of the placement-pool line. Work asks the pool for a place with its needs and releases it when done;
the answer names a node to pin a child to (ADR 0052). It follows the permit pool's protocol (ADR 0058), with a node
chosen per grant.

```rust
system.send(&pool, PlacementMsg::ask("run-17", ["arm64", "docker"], &run_ref, "Placed")?).await?;
// run-17 gets {"pool": "runners", "holder": "run-17", "kind": "granted", "grant": 4, "node": "factory-mac-1"}
// … creates its container child pinned to factory-mac-1, and when done:
system.send(&pool, PlacementMsg::release("run-17", 4)?).await?;
```

## Decisions

1. **The permit pool's protocol, per node.** `Ask { holder, needs, to, messageType }`, `Release { holder, grant }`,
   `Cancel { holder }`, and `Notify`, `Failed`, `Stopped` for the pool's own use. A holder id is 1 to 200 characters,
   no NUL, unique across every node's holders and the line, used once per attempt (as a permit request); needs are up
   to 32 labels with the bounds of offers. **Answers have their own type**, `PlacementAnswer { pool, holder, kind,
   grant?, node?, reason? }`, sent to `to` as `messageType`: kind `granted` (with `grant` and `node`), `queued`, or
   `refused` (with `reason` from a fixed list: `noNode` when no node could ever meet the needs, `lineFull`,
   `poolFull`); the silence alerts keep `PlacementNotice`. Grants are numbered by one counter in the state that only
   goes up (`nextGrant`); `Release` frees a place only when holder and grant match, so a late release of an old grant
   frees nothing. A repeated `Ask` of a current holder answers `granted` again with the same grant and node; of a
   waiting one, `queued` again; both keep the first ask's needs and target.
2. **Placement: every need, most free places, then name.** A node fits when it is live, has every label of the needs,
   and has a free place (its offered places minus its holders). Among the fits, the one with the most free places
   wins; ties go to the smaller node id. The holder is recorded on that node with its grant.
3. **Refuse, queue or grant.** An ask no node in the pool could ever meet (no node, live or silent, has every need) is
   refused at once with that reason. An ask that only silent nodes, or full nodes, could meet is queued, in a line of
   at most `max_waiting` (a new setting in the state, 0 to 1000, set at the create; a full line refuses). Holders are
   capped too: at most `max_holders` per pool (a setting, 1 to 1000, as the permit pool caps its holders), so the
   state stays small; a pool at its cap queues. After every message (as the permit pool's `grant_waiting`), the pool
   grants waiting asks in line order, **skipping asks that cannot be placed now** so one ask for a rare label does not
   hold up the others; this differs from the permit pool's strict order (ADR 0058), so ADR 0068 records it. A waiting
   ask whose needs no node has any more stays queued until `Cancel` (a node may come back).
4. **Answers in turns of their own, an undelivered grant freed.** Each holder record keeps `holder`, `needs`, `to`,
   `messageType`, `grant`, `node` and `notified`. As in the permit pool: the answer to an `Ask` goes
   out in the same commit (a target that is gone fails the ask and records nothing); a grant from the line is sent by
   a self `Notify` in a turn of its own, and when that notification ends `failed`, the pool frees the grant if its
   holder was never told (`notified == false`), through the watch the pool keeps on itself. A `Failed` for any other
   message of the pool (an `Alert`, a `Check`, an `Ask` whose target was gone) changes nothing.
5. **Places shrink without revoking.** An offer with fewer places than holders keeps every holder (no new grants until
   holders release); a silent node keeps its holders (only `placement-drain-and-gone` decides what a gone node's
   holders learn). `Cancel` withdraws a waiting ask; for a holder it does nothing (`Release` frees a place).
6. **One contract change inside the line; ADR 0068.** The state gains `maxWaiting`, `maxHolders`, `nextGrant`, the
   line and the holders per node; `PlacementState::new` takes `max_waiting` and `max_holders`; the messages gain
   `Ask`, `Release`, `Cancel`, `Notify`, `Failed`, `Stopped` and the answer type `PlacementAnswer`. The pool is
   unreleased (ADR 0067: the line ships as one minor
   version), so this changes nothing for hosts.

## Tests

On virtual time with `TestActors`: a grant names the node with the most free places, ties by id; an ask no node could
meet is refused with `noNode`; a full line with `lineFull`; a pool at its holder cap queues; an `Ask` to a gone target
records nothing; out-of-bounds messages are refused for good; a new node or an offer with a new label grants a waiting
ask; one only a silent or full node could meet is queued and granted when the node comes back or a place
frees; a full line refuses; a rare-label ask does not block a common one behind it; a release with a wrong grant frees
nothing; a repeated ask gets the same grant; a grant from the line to a gone target is freed; fewer offered places keep
holders; `Cancel` of a waiting ask; the bounds; the state decode; contract and samples.

## Docs

The handoff adds asks, grants, releases and pinning to the guide's placement pool section. Nothing breaks.
