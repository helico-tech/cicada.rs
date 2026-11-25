# Proposal

## Why

Work that must run on a particular kind of machine (Factory's containers, its git mirror per machine, issue 0004)
needs a pool where nodes offer capacity and work asks for a place; the answer is a node to pin a child to (ADR 0052).
Factory built one (`pool.rs`, `member.rs`, about 900 lines); nothing in it is about containers (request from the Factory
session, 2026-11-25; the owner accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- A ready-made actor in `cicada-actors`, the placement pool, where a node offers its places: a set of labels (such as
  the processor kind from `info()`) and a number of places, renewed at an interval. A node not heard from for a time
  gets no new places, and one alert is raised for it.
- It builds on what exists: the permit pool's grants and waiting line (ADR 0058) and the heartbeat watch for silence,
  where they fit.

To decide in the design: the actor and its contract, the interval and the silence time, how it relates to the permit
pool (reuse, wrap, or a sibling), and whether it needs an ADR (likely yes).

## Out of scope

Asks and holders (the next changes).

## Order

After `container-orphan-alert`, before `placement-asks`.

## Impact

`crates/cicada-actors` (a new actor and its contract in `docs/reference/contracts`). Additive.
