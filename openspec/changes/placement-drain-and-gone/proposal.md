# Proposal

## Why

An operator takes a node out of service: drain (no new places; running work finishes) and undrain, and declares a
silent node gone, after which its holders are told (request from the Factory session, 2026-11-25; the owner accepted the
split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- `Drain` and `Undrain` of a node; `Gone` of a node that is silent (refused for a node that is heard from), which
  frees its places and tells its holders, which then decide what to do.

To decide in the design: the notice to holders and its contract, what a drained node's offers do.

## Out of scope

Moving running work.

## Order

After `placement-holders`, before `placement-member`.

## Impact

`crates/cicada-actors`. Additive.
