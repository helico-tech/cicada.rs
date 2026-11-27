# Proposal

## Lane

Lane: fast

## Why

The rustdoc of `MessageView::caused_by` (`crates/cicada/src/api/mod.rs`) says "The message whose turn sent this one;
null for a message sent from outside an actor". That is not the whole truth: an operator retry records the original
message, and a `Stopped` notice records the message whose turn stopped the actor (ADR 0015), while a `Failed` notice and
a stop from outside a turn record none. Found by the builder of `inspector-outcomes` (2026-11-27), whose pane follows
the store.

## What Changes

- The rustdoc of `MessageView::caused_by` (and of the store's field it maps, if it says the same) names these cases.
- `docs/openapi.json` is regenerated, as the field's description comes from the rustdoc.

## Order

After `inspector-actions`.

## Impact

Crates: cicada.

`crates/cicada` (rustdoc only) and `docs/openapi.json` (regenerated). No behaviour change.
