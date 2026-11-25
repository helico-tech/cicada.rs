# Proposal

## Why

A holder that is deleted without a `Release` would keep its place for good. Factory hit this (issue 0014), which
`watch-absent-target` fixes in the core (request from the Factory session, 2026-11-25; the owner accepted the split
(CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- The pool watches each holder with `watch_existing`, so a holder deleted before or after its grant frees the place,
  exactly once.

## Out of scope

Holders on other systems.

## Order

After `placement-asks`, before `placement-drain-and-gone`.

## Impact

`crates/cicada-actors`. Additive.
