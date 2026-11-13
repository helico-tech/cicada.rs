# Proposal

## Why

A host that shows many actors, such as a dashboard, cannot list them with their state. `ActorSystem::list_actors` and
`GET /actors?prefix=` return addresses only, so a page of N actors costs one listing and N reads, on every refresh.

Found in cicada-code-factory: its showcase page lists every `factory.ticket` once a second, then reads each one.
That is fine at tens of tickets and wasteful at hundreds; every host with a board or a table writes the same loop.

## What Changes

- The listing of actors can return each actor's state and version with its address, paged by address as now
  (keyset, `next` cursor). Without the option it answers as today.
- The same over HTTP on `GET /actors` and in `cicada-client`.
- The simplest version: one option on the existing listing, read from the columns the store already has; no schema
  change.

## To decide

- An option on the existing call and route (`state=true`), or a call and route of its own (a list of snapshots).
- The page size with states: the same default and maximum as for addresses, or smaller, as states can be large.
- What a row holds besides the state: the version, `createdAt`, `paused`, `parent` (as `GET /actors/{type}/{key}`).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `management-api` (probably also `actor-state`): the listing of actors can carry each actor's state.

## Out of scope

Filtering, sorting or querying by fields of the state; projections or read models; a change feed (the next change,
`actor-change-feed`); a snapshot across pages (paging stays keyset, as now).

## Order

After `otel-timer-traces`, before `actor-change-feed`.

## Impact

The listing of both store backends and the conformance suite, `ActorSystem::list_actors` or its sibling, the
`GET /actors` route of `cicada-axum` and `docs/openapi.json`, `cicada-client`, `docs/guide/http.md` and the core
guide, the changelog. Once it is released, cicada-code-factory reads its board in one call (`src/ui/mod.rs`).
