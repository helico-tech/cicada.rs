# Docs handoff for placement-drain-and-gone

Version: 0.24.0 (unreleased; the placement line ships as one minor version). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

An operator takes a node of the placement pool `cicada.placement/<pool>` out of service, and declares a silent node gone.

- `PlacementMsg::drain(node)` (`Drain { node }`) sets the new node field `draining` (`PlacementNode::draining`, absent
  in a decoded state: false). A draining node gets no new places: it counts as no free places. It keeps its holders,
  still renews its offers, and can go silent and come back; an offer keeps the flag. An ask that only the draining node
  could meet is queued, not refused with `noNode`.
- `PlacementMsg::undrain(node)` (`Undrain { node }`) clears the flag and grants the waiting asks that fit.
- `PlacementMsg::gone(node)` (`Gone { node }`) works only for a node whose status is `silent`: it removes the node,
  frees all its places and tells each of its holders a `PlacementAnswer` of the new kind `gone`
  (`PlacementAnswerKind::Gone`), with its `grant` and `node`, sent to the holder's target as its message type, each in a
  `TellGone` turn the pool sends itself. A later `Release` of those grants frees nothing. Waiting asks stay in the line.
  The node offering again is a new node: a fresh entry, not draining.
- Refused for good, state unchanged: `Gone` of a live node (also one past its silence whose check has not run yet),
  `Drain`, `Undrain` or `Gone` of an unknown node, a node id out of bounds, and a `TellGone` from any sender but the pool.
  Draining a draining node, or undraining one that is not, changes nothing.

On the wire: `{"type":"Drain","node":"factory-mac-1"}`, `{"type":"Undrain",…}`, `{"type":"Gone",…}`; the answer
`{"pool":"runners","holder":"run-9","kind":"gone","grant":3,"node":"factory-mac-2"}`. The contract reference
`docs/reference/contracts/cicada.placement.json` is regenerated. Proved by `crates/cicada-actors/tests/placement.rs`:
`a_draining_node_gets_no_new_places_and_keeps_its_holders_until_undrain`,
`an_offer_keeps_the_drain_and_repeats_change_nothing`, `drain_undrain_and_gone_of_an_unknown_node_are_refused`,
`gone_of_a_live_node_is_refused_and_changes_nothing`, `gone_of_a_silent_node_frees_its_places_and_tells_each_holder_once`,
`a_gone_target_does_not_stop_the_other_holders_answers` and `a_tell_gone_from_another_sender_is_refused`; wire forms in
`tests/wire_formats.rs`. The doc example of `PlacementMsg::drain` shows its wire form.

## 1. Drain, undrain and gone in the guide's placement pool section

**Where.** `docs/guide/actors.md`, `Placement pool`: a new paragraph after the one on releasing, such as
`**Taking a node out of service.**`
**Text.** Drain a node to stop new places on it while running work finishes:
`system.send(&pool, PlacementMsg::drain("factory-mac-1")?)`. A draining node keeps its holders and its offers, and the
drain outlives a restart of the member: its offers do not clear it. Asks that only that node could meet wait in the line.
`PlacementMsg::undrain("factory-mac-1")?` gives it places again and grants the waiting asks that fit.
When a node's machine will not come back, declare it gone once its status is `silent`:
`system.send(&pool, PlacementMsg::gone("factory-mac-2")?)`. The pool removes it, frees its places, and tells each of its
holders `{"kind": "gone", "grant": 3, "node": "factory-mac-2", …}`. The holder decides what to do: start the work again
with a fresh holder id elsewhere, or fail it. A `Release` of that grant afterwards frees nothing. `Gone` of a live node
is refused: a node that is heard from is not gone; drain it instead.
**Caveats.**
- `gone` means "your place is no longer counted"; what ran there may still run, for example when the machine was only
  cut off. A holder that starts again elsewhere must cope with two copies of the work.
- A holder granted from the line and not yet told (its `granted` answer was still to come) can hear `gone` for a grant
  it never saw; its `granted` answer then never comes. Holder actors handle a `gone` for an unknown grant.
- Each `gone` answer goes in a turn of its own: a holder whose target is gone fails only its own answer.
- Waiting asks are untouched by `Gone`; one that only the gone node could meet waits, as for any node that has its
  needs no more.
- The node offering again after `Gone` is a new node: not draining, with no holders.
- The operator commands for drain and gone come with `placement-operator-commands`; until then, send the messages.

## 2. Answer kinds in the guide

**Where.** `docs/guide/actors.md`, `Placement pool`, where the answer kinds `granted`, `queued` and `refused` are
listed.
**Text.** Add `gone`: the holder's node was declared gone; `grant` and `node` say which place. Hosts that match the
kinds exhaustively need an arm for it.

## 3. The ready-made actors list in the architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crates table, the placement pool entry.
**Text.** Add: an operator drains a node (no new places, holders kept) and declares a silent node gone, which frees its
places and tells each holder with a `gone` answer in a turn of its own.

## Blocking

None.

## Status

Proposed text only; nothing protected was edited. Checks: see the report of this build.
