# Proposal

## Why

A host that shows many actors, such as a dashboard, cannot list them with their state. `ActorSystem::list_actors` and
`GET /actors?prefix=` return addresses only, so a page of N actors costs one listing and N reads, on every refresh.

Found in cicada-code-factory: its showcase page lists every `factory.ticket` once a second, then reads each one.
That is fine at tens of tickets and wasteful at hundreds; every host with a board or a table writes the same loop.

## What Changes

- Add ActorSystem::list_actors_with_state returning a page of the existing ActorSnapshot, with address, state,
  version, created_at, paused and parent. Keep the existing address-only listing unchanged.
- Add GET /actors/states and ActorClient::list_actors_with_state, returning pages of the existing ActorView. A
  separate route keeps the old route's response fixed and avoids a boolean query option and response union.
- Read each page with one actor SELECT per SQL read attempt, or one memory-store lock, using existing stored columns.
  Reuse literal address prefixes, keyset cursors and HTTP page limits (default 100, maximum 1000); core keeps its positive-limit check.
- Rows in one page share a read snapshot; there is no snapshot across pages. Reuse the single-actor read/error rules,
  including older deep states. ADR 0033 records the additive wire contract; there is no schema change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `management-api`: a host can list actors with their complete stored state snapshots.

## Out of scope

Filtering, sorting or querying by fields of the state; projections or read models; a change feed (the next change,
`actor-change-feed`); a snapshot across pages (paging stays keyset, as now).

## Order

After `otel-timer-traces`, before `actor-change-feed`.

## Impact

The private listing method of all three stores and the conformance suite, the new ActorSystem method, the
`GET /actors/states` route of `cicada-axum` and `docs/openapi.json`, `cicada-client`, core/HTTP guides, architecture,
ADR 0033 and changelog. No CLI or terminal UI change. After release cicada-code-factory can use one page read for its
board (`src/ui/mod.rs`); the host update is outside this change.
