Verdict: ready for tasks

## Blocking findings

None. Checked against the code: the SQLite and Postgres `claim_due` filter in SQL before `LIMIT`/`SKIP LOCKED`, so a pin predicate fits there. `sleep_time` and `ActorSystem::next_due_at` call `Backend::next_due_at(types)`, so the private trait gains an owner argument. `TurnInfo` is built in the shared executor, so `Turn::node_id` is feasible. `support.rs:43` already checks child flags per address; the pin check extends it. `NodeId` already rejects blank ids. `CreateActorRequest` and `ActorCreateView` use `deny_unknown_fields`, so old servers refuse `pinnedTo`, as the design says.

## Notes

1. Index (design §9): stranded pinned actors stay due forever. Every claim and next-due read on other nodes scans past them on `actors_due (type, next_due_at)`. Decide in migration 8: add `pinned_to` to an index, or name the cost per stranded row.
2. Fixtures (§11): "pin cases where useful" is weaker than ADR 0047, which requires lease cases for all alternatives. Require a valid pinned create, an invalid-pin failure and a coverage entry `TurnEffectsView.creates(pinned)`.
3. §9, "older HTTP workers must not be assigned pinned identities…": nobody can test this, and it is probably not needed. The new server filters by owner, so an old worker can run its own pins. Drop it or restate it.
4. Ready-made actors (supervisor `create_child_raw`, flow, event) cannot create pinned actors. A supervised process actor cannot be pinned. Name this under Out of scope, and in `pinned-process-example`.
5. §6, "returned/read snapshot": HTTP `CreateActorResponse` has only `{address, created}`; only core `Exists(ActorRecord)` returns one. Say that HTTP callers must GET. Name which error each surface returns for a too-long or NUL pin (OpError, TurnError, 400). Size is at the 5-task limit.

## Owner questions

None.

## Not checked

The TUI code, the CLI listing code, the details of the Postgres next-due query, the memory store, the OpenAPI generation and the e2e suite. No gate run. `openspec validate pinned-actors --strict` passes.
