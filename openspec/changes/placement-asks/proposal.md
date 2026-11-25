# Proposal

## Why

Work asks the pool for a place with its needs and releases it when done (request from the Factory session, 2026-11-25;
the owner accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- `Ask` and `Release` by a holder id with needs (a set of labels). A place goes to a node that has every need, most
  free places first, ties by name; asks that must wait queue in a bounded line; an ask is refused at once when no node
  in the pool has the needs, and waits when only silent or draining nodes have them. The answer names the node, to
  pin a child to (ADR 0052).

To decide in the design: the messages and replies, the line's bound, fairness in the line, an ask's time limit.

## Out of scope

Watching holders, drain.

## Order

After `placement-offers`, before `placement-holders`.

## Impact

`crates/cicada-actors`. Additive (the pool's contract grows before its first release, or a contract change after).
