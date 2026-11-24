# Proposal

## Why

The ready-made rate limiter spaces out starts, but nothing caps how many jobs are running at once. A build farm, a
pool of licensed workers or a host with room for N containers needs that cap. Issue #2 (higher-level actors) names
capacity reservations as the first of three candidates.

cicada-code-factory already has one: `factory.limiter` (`src/actors/limiter.rs`, about 115 lines), modelled on
`examples/src/saga/inventory.rs`. At most `max` runs hold a slot, and the rest wait in FIFO order. A repeated
`Acquire` from a holder answers `Granted` again. A `Release` is idempotent and also takes a run out of the queue. An
`Audit` asks every holder and waiter to check in, so a slot held by a run that is gone comes back. Each grant goes to
the run's actor in the same commit as the change that made room.

Issue #2 adds two rules the factory gets from its fresh run keys:

- A grant is keyed by a request ID, so a repeated acquire returns the same grant.
- A stale release must never free a newer grant to the same holder.
- When a holder disappears while its outside work goes on, capacity comes back only after the work is confirmed
  stopped, or the stale grant is fenced at the resource. The actor does not free a slot on a timeout alone.

## What Changes

- A ready-made permit pool in `cicada-actors`, one actor per pool, under the fixed type name `cicada.permits`.
- Messages: `Acquire` (a request ID and the target to tell), `Release` (the request ID, and the grant it releases),
  `Cancel` (leave the queue), `Resize` (change the capacity) and `Audit`.
- State: the capacity, the grants keyed by request ID with a grant number that only goes up, and a bounded FIFO
  queue. A full queue refuses an `Acquire` and says so.
- A grant is a notice sent to the requester's target: in the `Acquire`'s commit when there is room, otherwise in a
  turn of its own right after the commit that made room (design decision 7).
- Its contract is checked like the other ready-made actors' (`docs/reference/contracts`).
- Tests on virtual time with `TestActors`: a repeated acquire, a stale release, cancel while waiting, a full queue,
  FIFO order, and an audit that frees a slot only on a confirmed stop.

The decisions on the open questions are in [design.md](design.md).

## Out of scope

Inventory holds with confirmation and expiry, which issue #2 names as a later extension. Weighted permits, priorities
and fair sharing between tenants. A lease that frees a permit on a timeout alone. Moving the factory onto the pool.
Job graphs (`job-graph`).

## Order

After `container-actor`, before `job-graph`.

## Impact

A new module in `crates/cicada-actors` and its tests; a new contract file in `docs/reference/contracts`. Additive, so a
patch release. An ADR, as for each ready-made actor (ADR 0028). The docs handoff carries a section for the actors
guide and a skill route; nothing that exists breaks. Link the result in issue #2.
