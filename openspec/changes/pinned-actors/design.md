# Design

The owner delegated these remaining designs on 2026-11-16, overriding the two-ready queue limit and selecting
Opus 5.5 reviews. The process example is a separate immediate successor. Five tasks plus finishing cover this
placement primitive. ADR 0052 qualifies S8 and the wire/API contract; G1-G6 and all other store rules remain.

## Placement and identity

1. Store nullable pinned_to on the actor row, immutable for that actor life, not in its canonical address. No node
   foreign key or node-existence check: offline/not-yet-started targets are valid. Pinning says which claim owner is
   eligible, not which physical machine is authenticated. Keep NodeId opaque, nonblank, <=200 characters and NUL-free.
2. claim_due selects only hosted/requested types with pinned_to IS NULL or pinned_to == owner. Apply the predicate
   before candidate LIMIT, locking, epoch/attempt/lost-claim increments and claim-limit processing in every backend.
   Local nodes use NodeConfig.node_id; external leases use the supplied worker id, not the serving node's id.
   A matching worker needs no nodes row. Existing lease/renew/release/commit fencing and trusted-executor rules stay.
3. Qualify S8: progress requires an eligible executor that continues polling the type, in addition to existing
   due/unpaused/lease conditions. With no such executor, messages wait indefinitely without new outcomes or counters.
   next_due_at and the node's internal sleep calculation use the same owner filter; foreign pins must not cause a
   busy loop. Unpinned actors retain existing behavior. Bounds, ordering and fairness remain as before among eligible rows.
4. The host configures a stable, globally unique id for placement across restart (existing --node-id/CICADA_NODE_ID).
   A default random id will not recover another id's pins. Duplicate ids may run the same pinned actors on different
   machines under ordinary lease exclusion. No liveness from heartbeat status, node registration or machine label,
   no failover/reassignment, and no protection from an outside effect continuing after a handler loses its lease.

## Creation and public forms

5. Add public CreateOptions { pinned_to: Option<NodeId> }, default None. Keep existing methods/signatures unpinned;
   add ActorSystem create_with/create_actor_with/create_actor_checked_with and client create_with/create_actor_with,
   with options before the check callback for checked create. Add Turn create_with/create_child_with and
   create_raw_with/create_child_raw_with. Preserve existing typed/raw schema/decode checks and error mappings.
   Options are validated before any side effect. Expose Turn::node_id() as the actual executing node/worker id,
   injected by the shared executor, so create_child_with can pin explicitly to that id. Children never inherit pins.
6. Carry pinned_to through ActorCreate, EffectBuffer, returned reports and store commit creates, atomic with the
   existing effects. For an existing actor, any create keeps its original state/parent/pin, including a different
   requested pin; retain foreign-child rejection. Core callers inspect the snapshot where returned; HTTP create
   returns only address/created, so HTTP callers must GET the actor to verify its pin.
   In one turn, repeated creates for an address must agree on child flag AND pin, including None versus Some;
   otherwise TurnError::Invalid without buffering the conflicting call, or StoreError::Invalid for a supplied commit
   before lease checks. Valid same-pin duplicates retain current first-create semantics. Delete/recreate is a new life.
7. Add optional pinnedTo to CreateActorRequest and ActorCreateView (remote completion creates), default None on
   missing/null and omit None on serialization, keeping old request/fixture bytes. Reject invalid ids/unknown fields
   as OpError::Invalid in core creates/completion, TurnError::Invalid when buffering, StoreError::Invalid in direct
   store requests, and RequestError in wire-effect conversion; HTTP maps these to 400 bad-request. CLI rejects
   invalid flag values as usage errors. Add defaulted optional pinnedTo to ActorView and pinned_to to
   ActorSnapshot/ActorRecord. Views serialize null for unpinned and string for pinned. No new lease-grant field: the
   requesting worker already knows its owner id. Generated OpenAPI documents the new fields/constraints.
8. Add create actor --pinned-to ID and display PINNED-TO in wide actor-state listings and describe actor, JSON included.
   The same command works against local/HTTP backends. Use existing paginated state listings to find a pin and existing
   mailbox listing to inspect its backlog; no metadata duplicated into every message row, node-detail fanout or new
   pin filter. Direct deletion/purge/cancel controls still work from any node. CLI TUI views consuming ActorView must
   retain/display placement when showing actor detail; no new screen. Pin is independent of pause and parenthood.

## Storage, compatibility and verification

9. Append migration 8 in SQLite/PostgreSQL, nullable text pinned_to default NULL; memory store mirrors it. Preserve
   state, parent, epochs, messages and all old rows as unpinned. Add a partial index on (pinned_to, type, next_due_at)
   for nonpaused rows with next_due_at, retaining actors_due; use it for eligible-range planning without promising
   a particular planner choice. No migration rewrite or address/contract-hash change.
   The new schema is NOT safe with old open direct-store nodes: their claim query ignores pins. Stop/drain every old
   direct-store runtime before migrating/restarting; upgrade all HTTP servers before sending pinned creates/effects.
   Older HTTP workers can process matching pins: upgraded servers enforce eligibility. Their older create APIs
   cannot express pins. Old strict servers refuse pinnedTo rather than enforce it.
10. Regression tests cover core/typed/raw/child and checked create, duplicate pins, rollback, existing actor no-op,
    validation before stale commit, snapshots, migration, delete/recreate, and independent parent/child pins. Conformance
    checks eligible selection before limit, all-foreign empty batch/counter preservation, next due, pause, expired
    leases/reclaim and unchanged unpinned order on memory/SQLite/Postgres. Extend fuzz model with pin eligibility.
11. Test local and HTTP workers with matching/mismatched owner versus server id and Turn::node_id, lost-claim limits,
    and no node-row prerequisite. Runtime tests use paused time/TestActors. Check client/HTTP JSON/schema errors, CLI
    local/remote output and generated help/OpenAPI. Keep old portable case bytes unchanged (omitted None); require a
    valid pinned-create lease case, invalid-pin conversion failures and TurnEffectsView.creates(pinned) coverage.
    Add cases without regenerating old expectations. Do not promise backward Rust struct
    literal compatibility: new public fields require downstream updates, named in upgrading guidance.

## Documentation and finishing

12. Builder hands docs owner architecture Scope/Domain language/Store requirements S8/Limitations and schema-upgrade
    text, core actor creation/children/worker guidance, HTTP create/remote-completion fields, operations stable ids and
    pending-work diagnosis, app CLI usage, upgrading stop/drain instructions and docs/guarantees S8 proving tests.
    Guide design's identity/outside-system sections explain routing versus handles; the process example stays next.
    Preserve anchors; skills route to existing headings. Rust snippets with public struct literals may need exact
    owner-prepared fixes before landing. Record all broken blocks in docs-handoff.md; no skipped docs/skill checks.
13. Builder owns ADR/index, source/rustdoc, appended migrations, tests, generated CLI/OpenAPI, changelog and normal
    release pins. Verify targeted conformance/runtime/worker/client/CLI/fixture tests, docs/doctests/skill and strict
    OpenSpec; finish with committed-tree PostgreSQL/Docker gate. No process management or node-down alarms here.
    Ready-made supervisor/flow/event creation protocols gain no pin options; the next example creates pins directly.
