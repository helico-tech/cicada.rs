# Docs handoff for placement-full-notice

Version: 0.25.0 (unreleased batch). Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

A placement pool that knows `maxNodes` nodes or more no longer refuses an offer from a node it does not know (ADR 0075,
crate `cicada-actors`). It accepts the offer's turn and ignores it: the turn commits, the node is not added and no entry
changes. Before, such a member's offer became a dead letter at every interval and nobody was told.

- The pool keeps one mark in the state's new optional field `full`, the public `PlacementFull { node, at }`: the node of
  the last ignored offer and that offer's store time. On the wire `"full": {"node": "mac-3", "at": "..."}`, written
  only while it is set; states without it decode.
- When the mark is set, the pool sends the alert target one notice of the new kind `full`
  (`PlacementNoticeKind::Full`, `"full"` on the wire), about that first ignored node. Later ignored offers, from any new
  node, only move `node` and `at` and send nothing.
- The mark ends one silence time after its `at` (the pool's check removes it, like a contest), or at once when an offer
  adds a node, after `Configure` raised `maxNodes` or a `Gone` freed a slot. So a pool that fills up again alerts again.
- An ignored offer arms the check when none is armed (a full pool whose nodes are all silent). `armed` and `armedId` are
  now set while a node is live, a contest is listed or the pool is marked full; a decoded state with a mark and no armed
  check is refused.
- The resource check of ADR 0072 comes first: a new node offering a key another live node holds is still a contest with
  its `conflict` alert, full pool or not. A known node offers as before. A new contest past `maxNodes` stays refused for
  good.
- `PlacementNoticeKind` is now `#[non_exhaustive]`: a host's `match` on it needs a wildcard arm.

Proved by `crates/cicada-actors/tests/placement.rs` (`a_new_node_offering_to_a_full_pool_is_ignored_and_alerted_once`,
`a_known_node_in_a_full_pool_offers_as_before`,
`the_full_mark_ends_one_silence_time_after_the_last_ignored_offer_and_a_later_one_alerts_again`,
`a_larger_node_limit_admits_the_next_new_node_ends_the_mark_and_a_pool_full_again_alerts_again`,
`a_new_node_offering_a_held_key_to_a_full_pool_is_a_contest`,
`a_full_pool_whose_nodes_are_silent_arms_the_check_on_an_ignored_offer_and_the_check_ends_the_mark`, and the changed
`offers_out_of_bounds_are_refused` and `a_lower_node_or_line_limit_keeps_what_is_there_and_refuses_new_ones`), the unit
test `a_decoded_full_mark_is_checked_and_keeps_the_check_armed` and `crates/cicada-actors/tests/wire_formats.rs`. The
contract reference `docs/reference/contracts/cicada.placement.json` is regenerated. ADR 0075 and its row are in this
change.

## 1. The `full` notice in the alerts paragraph

**Where.** `docs/guide/actors.md`, `Placement pool`, the paragraph `**Alerts.**` ("with the kind `silent` once per
silence, `back` when ... and `conflict` once while a node's conflict mark lasts; match all three if you match the kinds
exhaustively").
**Text.** Add the fourth kind: `full` once per full period of the pool, about the first new node it ignored then (see
the new paragraph below). Replace "match all three if you match the kinds exhaustively" with: `PlacementNoticeKind` is
`#[non_exhaustive]` from 0.25.0, so a `match` on it needs a wildcard arm, and later kinds do not break it.
**Caveats.** During a rolling upgrade an alert target on an older node does not know `full` and fails that alert for
good; nothing else fails.

## 2. A new paragraph: a full pool

**Where.** `docs/guide/actors.md`, `Placement pool`, a new bold-titled paragraph such as `**A full pool.**`, near the
settings paragraph (`**Changing the settings.**`, from placement-settings' handoff) or `**Fewer places.**`.
**Text.** When the pool knows `maxNodes` nodes or more, an offer from a node it does not know is ignored (from 0.25.0,
ADR 0075, linked as `../adr/0075-a-full-pool-says-so-once.md`): the turn succeeds and the node is not added; the offer
changes no node entry (the turn still does the pool's usual work, such as ending a conflict mark whose time is up). The pool marks itself full in the state's `full` (`{node, at}`: the last ignored node and when) and tells the
alert target once with the kind `full`, naming the first ignored node; further ignored offers, from any new node, only
move the mark. The mark ends one silence time after the last ignored offer, or as soon as an offer adds a node, so a
pool that fills up again is told again. To let the node in, raise `maxNodes` with `Configure`
(`cicada send cicada.placement/runners --type Configure --payload '{"maxNodes": 20}'`) or declare silent nodes `Gone`;
the node is added at its next offer. A new node offering a resource key another live node holds is a contest first,
with its `conflict` alert, and not counted as a full-pool offer. A known node renews as before, also in a full pool,
unless it offers a resource key another live node holds (a contest, as always). An ignored offer's turn still does
the pool's usual work of a turn, such as ending a report mark whose time is up.
**Caveats.** One mark per pool, not per node: the notice names only the first ignored node of the period; read `full`
in the state for the latest. The pool cannot say how many nodes were turned away.

## 3. The refusals sentence

**Where.** `docs/guide/actors.md`, `Placement pool`, the paragraph after `**A lost check.**` ("A message out of bounds,
and an offer from a new node when the pool knows `maxNodes` nodes, is refused for good with the state unchanged; a
known node can always renew.").
**Text.** Drop the new-node clause: "A message out of bounds is refused for good with the state unchanged; an offer
from a new node to a full pool is ignored with one `full` notice (see **A full pool**); a known node's offer is
handled as before (renewed, or ignored under an instance conflict or a resource contest)."
**Caveats.** The placement-settings handoff may already have reworded this sentence for lower limits ("new ones are
refused (a new node, ...)"): a new node is now ignored, not refused; a new contest and an ask that must wait in a full
line are still refused.

## 4. The settings paragraph from placement-settings

**Where.** `docs/guide/actors.md`, the `**Changing the settings.**` paragraph, if written from placement-settings'
handoff ("new ones are refused (a new node, a new contest, an ask that must wait while the line is full)"; "a higher
`maxNodes` admits a new node at its next offer").
**Text.** A new node past a lower `maxNodes` is ignored with the `full` notice, not refused; a higher `maxNodes` admits
it at its next offer, which ends the full mark. A new silence also moves the end of the full mark.
**Caveats.** None.

## Blocking

None. The guide's doctest of the pool matches no notice kind exhaustively, so `#[non_exhaustive]` breaks no block.

## Status

Ran `cargo test -p cicada-examples --test docs` (11 passed) and `--doc` (51 passed), and the finishing checks: fmt,
clippy, `cargo test --workspace` (3,735 passed, 0 failed) and the gate (full pass of f16bf210). All
the text above is proposed; no protected prose is written yet.
