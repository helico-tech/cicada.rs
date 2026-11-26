# Proposal

## Why

Two runner nodes on one machine, each with its own identity, both offer the same Docker daemon, so its places count
twice (request from the Factory session, 2026-11-26, its epic 0008; blocks Factory's merge).

## What Changes

- `Capacity` (and `Offer`) carry an optional resource key, such as the Docker daemon's id; the pool ignores a second
  live node that offers a key another live node holds, and sends one `conflict` notice.
- `RuntimeInfo` exposes Docker's daemon id (`info().ID`), so a host can use it as the key.

To decide in the design: the key's bounds, which node keeps the key (the first live one), the end of the conflict (as
ADR 0069's mark), and the `RuntimeInfo` field on the fake.

## Order

After `placement-identity-lock`, before `placement-settings`.

## Impact

`crates/cicada-actors` (offer, pool state and contract), `crates/cicada-containers` (`RuntimeInfo`, Docker, fake).
Additive.
