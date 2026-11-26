# Design

The fourth change of the placement-pool line. An operator takes a node out of service (drain, undrain) and declares a
silent node gone, after which its holders are told and its places freed. Factory has both today.

```rust
system.send(&pool, PlacementMsg::drain("factory-mac-1")?).await?;    // no new places; running work finishes
system.send(&pool, PlacementMsg::gone("factory-mac-2")?).await?;     // only for a silent node: its holders hear Gone
// each holder of factory-mac-2 gets {"pool": "runners", "holder": "run-9", "kind": "gone", "grant": 3,
//   "node": "factory-mac-2"} and decides what to do (start again elsewhere, fail)
```

## Decisions

1. **Drain is a flag on the node.** `Drain { node }` and `Undrain { node }` set and clear `draining` on a known node
   (unknown: refused for good). A draining node gets no new places, keeps its holders, and still renews its offers and
   can go silent and come back; an offer does not clear the flag (an operator decision outlives a restart). Where
   `placement-asks` counts free places, a draining node counts as none; the check that refuses an ask no node could
   ever meet still counts draining nodes, so such asks are queued, not refused. Undrain scans the line. Draining a
   draining node, or undraining one that is not, changes nothing.
2. **Gone only for a silent node.** "Silent" is the node's status `silent` (set by the pool's check); a node past its
   silence time whose check has not run yet is still `live`. `Gone { node }` is refused for good for a live node (the
   node is heard from, so it is not gone) and for an unknown one. For a silent node it removes the node, frees all its
   places, and tells each
   holder: a `PlacementAnswer` of kind `gone` with its grant and node, sent to the holder's target as its message type,
   each in a turn of its own (a self `TellGone { holder, grant, node, to, messageType }`, carrying what it needs as the
   holder records go at once, so one failing target does not stop the others; a `TellGone` from anyone else is refused
   for good); a late `Release` frees nothing. A holder granted from the line and not yet told can hear `gone` for a
   grant it never saw: the handoff warns holder authors. Waiting asks are untouched (they may fit elsewhere);
   the line is scanned. A node that offers again after `Gone` is a new node (fresh entry, not draining).
3. **Answers stay one type.** `PlacementAnswer` gains the kind `gone`; nothing else changes. The handoff tells holder
   authors that `gone` means "your place is no longer counted; what ran there may still run".
4. **Additive inside the line.** New messages `Drain`, `Undrain`, `Gone`, `TellGone`, the node field `draining` and the
   answer kind `gone`; it ships with the line in 0.24.0. The operator commands for these come in
   `placement-operator-commands`.

## Tests

On virtual time with `TestActors`: a draining node gets no new places and keeps its holders; an offer keeps the flag;
undrain grants a waiting ask; `Gone` of a live node and of an unknown node are refused; `Gone` of a silent node frees
its
places, tells each holder once with `gone`, and a late `Release` frees nothing; one holder whose target is gone does not
stop the others' notices; the node offering again is a new node; contract and samples.

## Docs

The handoff adds drain, undrain and gone to the guide's placement pool section, with what a holder should do on `gone`.
