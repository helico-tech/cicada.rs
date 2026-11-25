# Proposal

## Why

An operator drains, undrains, declares gone and releases from the command line (request from the Factory session, 2026-11-25; the owner accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- Operator commands of `cicada-app` (or a documented way through the existing send command) for drain, undrain, gone
  and release, over HTTP like the other operator commands, with `docs/reference/cli.md` regenerated.

To decide in the design: dedicated commands or the generic send, how the pool is named.

## Out of scope

A terminal UI page.

## Order

After `placement-member`. No following change.

## Impact

`crates/cicada-app`, `docs/reference/cli.md`. Additive.
