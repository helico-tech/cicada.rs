# Docs handoff for placement-conflicts

Version: 0.24.0 (unreleased; the placement line ships as one minor version). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

A second machine offering under a live node's id of the placement pool `cicada.placement/<pool>` is ignored and
reported once (ADR 0069).

- An `Offer` whose `instance` differs from the node's recorded one while the node is `live` changes no entry field
  (labels, places, `heardAt` stay the first machine's). The turn succeeds: it sets the new node field `conflict`
  (`PlacementNode::conflict`, a `PlacementConflict { instance, at }`: the other instance and the store time of that
  offer). When the node had no mark, the pool sends one `PlacementNotice` of the new kind `conflict`
  (`PlacementNoticeKind::Conflict`) in an `Alert` turn of its own. Later conflicting offers, from the same or a third
  instance, only update the mark: no new alert. No count of ignored offers is kept.
- The mark ends when the node goes `silent` (the first machine stopped), or when the recorded instance offers one
  silence time or more after the mark's `at` (the second machine stopped). A later conflict then alerts again.
- A silent node's entry is taken over by the next offer, whatever its instance (a replaced machine), with a `back`
  notice, as before.
- A decoded state with `conflict` on a node that is not live, or naming the recorded instance, is refused.

On the wire: the node field `"conflict": {"instance": "b2c4", "at": "2026-01-01T00:00:10Z"}`, written only while set;
the notice `{"pool":"runners","node":"factory-mac-1","kind":"conflict"}`. The contract reference
`docs/reference/contracts/cicada.placement.json` is regenerated. Proved by `crates/cicada-actors/tests/placement.rs`:
`a_second_instance_under_a_live_node_changes_nothing_and_alerts_once`,
`a_third_instance_updates_the_mark_without_an_alert`,
`after_the_first_machine_goes_silent_the_second_takes_over_and_the_mark_clears` and
`after_the_second_machine_stops_for_one_silence_time_the_mark_clears_and_a_later_conflict_alerts_again`; the decode by
`a_decoded_conflict_is_allowed_only_on_a_live_node_and_names_another_instance` (unit test in `placement.rs`); wire forms
and samples in `tests/wire_formats.rs`.

## 1. Offers: two machines under one node id

**Where.** `docs/guide/actors.md`, `Placement pool`, the paragraph `**Offers.**`, and a new paragraph after it, such as
`**Two machines under one id.**`
**Text.** In `**Offers.**`, "Each offer replaces the node's entry" gets "…, except an offer of another instance while
the node is live (below)". New paragraph: When a second machine offers under the id of a live node with another
`instance`, the pool keeps the first machine's entry (labels, places, `heardAt`) and ignores the offer. It marks the
node `conflict: {instance, at}` with the other instance and the time of its last offer, and tells the alert target once
with the kind `conflict`. Further offers of other instances only update the mark. The mark ends when the node goes
silent, as the first machine stopped (the next offer, of any instance, then takes over the entry: a replaced machine),
or when the first machine offers one silence time after the last conflicting offer, as the second stopped; a new
conflict after that is told again. Fix it by giving each machine its own node id.
**Caveats.**
- The pool only reports. Two machines under one node id also share that node's leases and its node row in Cicada's
  store; the hosts must fix that.
- A cloned disk with the member's identity file copied looks like one machine: the pool cannot tell.
- Until the member (`placement-member`) keeps its instance across restarts, the instance is drawn at each process start,
  so a restart within the silence time counts as a conflict: the restarted node's offers are ignored until its old entry
  goes silent (alerts `conflict`, `silent`, then `back`).
- The notice does not name the other instance; read the node's `conflict` in the pool's state.

## 2. Alert kinds in the guide

**Where.** `docs/guide/actors.md`, `Placement pool`, the paragraph `**Alerts.**`.
**Text.** Add the kind `conflict`: another instance offered under a live node's id; once while the mark lasts. Hosts
that match the kinds exhaustively need an arm for it.

## 3. The ready-made actors list in the architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crates table, the placement pool entry, where
"`silent` once per silence, `back` at the next offer" is listed.
**Text.** Add `conflict` once when another instance offers under a live node's id (ADR 0069): its offers are ignored and
the node is marked until it goes silent or the other instance stops for one silence time.

## Blocking

None.

## Status

Proposed text only; nothing protected was edited. Checks: see the report of this build.
