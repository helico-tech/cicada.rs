# Proposal

## Why

When a pool holds `maxNodes` nodes, an offer from a new node is refused for good: a dead letter every interval and no
alert (request from the Factory session, 2026-11-26, its epic 0008).

## What Changes

- The pool accepts such an offer's turn, ignores it, and sends one notice of a new kind `full` per new node (as the
  conflict alert of ADR 0069), so there is an alert and no stream of dead letters.

To decide in the design: once per node or per pool, when the mark ends.

## Order

After `placement-settings`, before `app-node-id-after-types`.

## Impact

`crates/cicada-actors` (pool, contract). Additive.
