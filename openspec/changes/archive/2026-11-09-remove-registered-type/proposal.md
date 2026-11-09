# Proposal

Status: built on 2026-11-09 (agreed by Arjan on 2026-11-09). Depends on
`management-api`.

## Why

A registered type cannot be removed or reset. `docs/operations.md` tells operators to delete the row from `actor_types`
by hand in SQL while no node runs.

## What Changes

- `DELETE /types/{name}` removes a type's registration. It is refused (`409`) while actors of that type exist.

A node that still hosts the type keeps running and registers it again at its next start. Stop such nodes first; the
operations guide says so.

## Out of scope

- Deleting the actors of a type along with it.
- Migrating a contract in place.

## Impact

- A new store method in all three stores and new conformance cases.
- The resolver's contract cache must not keep serving a removed type.
- `cicada-axum`, `cicada-client`, `docs/operations.md` ("Changing a contract").
