# 0033. Listing actors with their state

Date: 2026-11-13

## Status

Accepted for `list-actors-with-state`. The owner delegated design decisions to an Opus 5.5 adviser.

## Context

A dashboard lists actor addresses, then reads every actor to obtain state. This costs N extra reads per page and can
observe deletes or updates between the address listing and each read. The store already has every required column.

## Decision

- Add ActorSystem::list_actors_with_state returning Page<ActorSnapshot>, reusing the complete single-actor snapshot.
  Keep list_actors unchanged. The private store interface gains a paged record read in all three backends.
- Add GET /actors/states, with the same prefix/cursor/limit parameters as GET /actors, and PageView<ActorView> as its
  fixed response. Serve it on both routers, with operationId listActorStates. Add the matching typed client method.
- Reuse address keyset order, literal prefix matching, HTTP default page size 100 and maximum 1000. Core keeps the existing positive-limit check. Each SQL page is one
  actor SELECT per read attempt; memory reads under one lock. State and version come from the same committed row and
  rows in a page share a read snapshot. There is no snapshot across pages, durable cursor position or version history.
- A row carries address, state, version, createdAt, paused and parent; the HTTP view also carries type, as the existing
  single-actor view does. Stored values follow existing read rules, including legacy deep values (ADR 0029).

## Consequences

- A page replaces N+1 actor reads with one set-based read. No change to G1–G6, S1–S10, actor contracts or SQL schema.
- A separate route avoids a response union on the existing listActors operation and a new boolean query parser.
  Existing clients keep their address-only response. New clients require a server with the new route.
- Large states can make large pages; clients choose a smaller limit. No new size cap or field projection is added.
  A corrupt fetched row fails the whole page, and one legacy deep state may exceed a client's JSON parser limit.
- An address inserted before the cursor is missed; deletion removes rows, and later pages see later updates. State
  versions are local to an actor life and can reset after recreation; they are not a global change-feed watermark.
- The architecture and guides describe the listing when implemented. Change notifications remain a separate change.
