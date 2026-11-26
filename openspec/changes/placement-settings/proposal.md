# Proposal

## Why

A pool's silence, node limit, line limit and holder limit are fixed at its create; an operator who needs another value
deletes and recreates the pool (request from the Factory session, 2026-11-26, its epic 0008).

## What Changes

- A `Configure` message (or one per setting) changes the settings of an existing pool, with the same bounds as the
  create; lowering a limit below what is held keeps what is held and refuses new ones until under it.

To decide in the design: one message or several, what happens to an armed check when the silence changes, the operator
recipe (`cicada send`).

## Order

After `tray-worker-control`, before `placement-full-notice`.

## Impact

`crates/cicada-actors` (pool messages, state checks, contract). Additive.
