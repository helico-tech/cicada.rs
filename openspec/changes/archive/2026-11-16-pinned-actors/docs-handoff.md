# Docs handoff for pinned-actors

Version: 0.20.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published. A breaking release
(public struct fields, store schema 8 that old direct-store nodes must be drained for), written so in `CHANGELOG.md`.

## What changed

An actor may be pinned at creation to one claim owner (ADR 0052): a node's `NodeConfig::node_id` or a worker's
`WorkerConfig::worker_id`. Only that owner claims its turns; every other owner skips it before the claim limit and
changes nothing of it, and its next-due read ignores it. Unpinned actors behave as before.

- Core: `cicada::CreateOptions { pinned_to: Option<NodeId> }` (also in the prelude), `CreateOptions::new().pinned_to(id)`.
  `ActorSystem::create_with`, `create_actor_with`, `create_actor_checked_with(address, state, options, check)`;
  `Turn::create_with`, `create_child_with`, `create_raw_with`, `create_child_raw_with`; `Turn::node_id()`.
  `ActorSnapshot::pinned_to`, `ActorRecord::pinned_to`, `ActorCreate::pinned_to`. Existing calls stay unpinned.
- Bounds: a pin is a non-blank `NodeId` of at most 200 characters with no NUL. Errors: `OpError::Invalid` (core create,
  `complete_turn`), `TurnError::Invalid` (turn calls), `StoreError::Invalid` (store), `RequestError` (wire conversion),
  HTTP `400`, CLI usage error (exit 2).
- Creates: an existing actor keeps its pin (and state, parent) on any repeated create; `Exists` holds its snapshot.
  Creates of one address in one turn must agree on child flag and pin (`None` against `Some` included), else
  `TurnError::Invalid` at the second call and nothing is buffered. Children never inherit a pin. Delete, then create,
  starts a new life that may choose another pin (also a restart in one commit).
- Execution: `ActorSystem::lease_turns` filters by the requesting worker's id, not the server's; the worker needs no
  node row. `Turn::node_id()` is the executor's id on a node and on a worker. A pinned actor whose owner never polls
  its type keeps its messages pending: no attempt, lost claim, failure or reassignment, and no busy dispatch loop.
- HTTP: optional `pinnedTo` on `CreateActorRequest` (`PUT /actors/{type}/{key}`) and on `ActorCreateView` (returned
  creates of `POST /leases/complete`), absent or null for none, left out when none (old bytes unchanged). `pinnedTo` on
  `ActorView` (`GET /actors/{type}/{key}`, `GET /actors/states`), `null` when unpinned; read as `None` when an older
  server leaves it out. `docs/openapi.json` regenerated (`maxLength` 200, `minLength` 1). The create response is still
  only `address` and `created`. `ActorClient::create_with`, `create_actor_with`.
- App: `cicada create actor --pinned-to ID` (regenerated `docs/reference/cli.md`); `PINNED-TO` in the wide actor table
  after `PARENT`; `pinnedTo` in `describe actor` and JSON; a `Pinned to` fact on the TUI's actor screen. No new list
  filter or node-detail expansion.
- Store: schema 8 on SQLite and PostgreSQL: nullable `actors.pinned_to` and partial index `actors_pinned_due` on
  `(pinned_to, type, next_due_at)` for unpaused rows with a due time, beside `actors_due`. Old rows unpinned.
- Fixtures: new cases in `fixtures/leases.json` (valid `complete-request-returned-pinned-create`, accepted
  `complete-request-create-pin-null`, invalid `complete-request-create-pin-too-long`, `-with-nul`, `-blank`,
  `-not-a-string`) and the label `TurnEffectsView.creates(pinned)`; every older case is byte-identical.
- Tests: `crates/cicada/src/tests/conformance/placement.rs` (five cases on memory, SQLite and PostgreSQL),
  `crates/cicada/src/tests/runtime/placement.rs` (node, worker, turn and outside creates, no-spin),
  `crates/cicada/tests/completion.rs::a_returned_create_applies_its_pin_and_bad_or_disagreeing_pins_are_refused`,
  the upgrade tests `a_database_of_schema_version_7_is_opened_and_its_actors_are_unpinned` (both stores), the fuzz
  comparison with pins, `crates/cicada-client/tests/client.rs::a_pinned_create_is_read_back_and_bad_pins_are_bad_requests`
  and `::a_worker_over_http_claims_under_its_own_id_not_the_servers`,
  `crates/cicada-app/tests/cli.rs::an_actor_created_with_a_pin_shows_it_and_its_messages_wait_in_its_mailbox`.

## 1. Architecture: scope, language, S8, schema and limitations

**Where.** `docs/architecture.md`: `Scope`, `Domain language`, `Store requirements` (S8 row), `Workers`,
`HTTP contract`, `Known limitations`; and the schema-version text wherever the architecture names version 7.
**Text.** Domain language: a *pin* is an optional, immutable claim owner of an actor, chosen at creation and separate
from its address and parent; a *claim owner* is the node id or worker id a claim is made for. Store requirements: the
claim admits an actor only when unpinned or pinned to the claim owner, before the claim limit and before any change
to the actor; `next_due_at` uses the same owner filter. S8 row: "An instance that is not paused, with a due head
message and no valid lease, is eventually returned by `claim_due` to a claimer that keeps polling its type and is
eligible for it: any owner when it is not pinned, else the owner it is pinned to (ADR 0052). A paused instance is not
claimed until it is resumed." Workers: a worker's leases are filtered by its `worker_id`, never the server's id;
`Turn::node_id()` is that id. HTTP contract: the optional `pinnedTo` request fields and the `pinnedTo` view field.
Schema: version 8 adds `actors.pinned_to` and `actors_pinned_due`.
**Caveats.** A pin is a routing identity, not an authenticated machine; there is no failover, no liveness from node
rows, heartbeats or labels, and no fence for an outside effect that outlives a lost lease. Limitation to add: a
pinned actor whose owner never returns waits forever; pins cannot be moved (delete and create again), and the
ready-made supervisor, flow and event protocols cannot create pinned actors.

## 2. Guarantees: S8 row and its tests

**Where.** `docs/guarantees.md`, the `S8 Liveness` row.
**Text.** The qualified statement of item 1, and these proving tests added to the row:
`crates/cicada/src/tests/conformance/placement.rs::placement_foreign_pins_never_take_the_place_of_an_eligible_actor`,
`crates/cicada/src/tests/conformance/placement.rs::placement_pause_and_expired_leases_apply_to_the_owner_as_to_anyone`,
`crates/cicada/src/tests/runtime/placement.rs::a_started_node_does_not_spin_on_a_spot_pinned_elsewhere`.
**Caveats.** `examples/tests/docs.rs` checks that named tests exist; the three names above exist on this branch.

## 3. Core guide: creating pinned actors, children and workers

**Where.** `docs/guide/core.md`: `Child actors`, `Open a store and run a node` (or a new subsection beside it, such as
`Pin an actor to a node`), `Run actors in a worker`, `Configuration`.
**Text.** Create a pinned actor from outside with `system.create_with(&r, &state, CreateOptions::new().pinned_to(id))`
and inside a turn with `turn.create_with(...)` / `create_raw_with`. To keep a child on the machine of its parent's
turn: `turn.create_child_with(&child, &state, CreateOptions::new().pinned_to(turn.node_id().clone()))`; a child is
never pinned implicitly. Use a pin when a turn must act on something only one machine can reach (a container, a
process); to only observe a machine, a local task that sends messages needs no pin. A pinned actor runs only on the
node or worker whose id equals the pin, so give that executor a stable, unique `node_id` / `worker_id`. An existing
actor keeps its pin; delete it and create it again to move it. A Rust example can be compiled from the runtime test
`only_the_node_a_spot_is_pinned_to_runs_it_and_the_others_do_not_wait_for_it` (create, send, run on node B).
**Caveats.** `Turn::node_id()` on a worker is the worker's id, not the HTTP server's. Duplicate ids give no exclusive
machine placement (one lease at a time still holds).

## 4. HTTP guide: create field, remote completion and views

**Where.** `docs/guide/http.md`: `The client`, `HTTP API`, `Errors and limits`, `Run a worker over HTTP`,
`List actors with their state`.
**Text.** `PUT /actors/{type}/{key}` takes optional `"pinnedTo": "host-a"`; a returned create of
`POST /leases/complete` takes the same field. `GET /actors/{type}/{key}` and the state listing serve `pinnedTo`
(`null` when unpinned). The create answers only `address` and `created`: a caller that needs the pin of an existing
actor GETs it. A blank, non-string, too long (over 200 characters) or NUL pin is `400 bad-request`; an unknown field is
still `400`. `ActorClient::create_with(&r, &state, CreateOptions::new().pinned_to(id))`. A worker over HTTP leases
under its own `worker_id`: it runs actors pinned to it and unpinned ones, never ones pinned to the server.
**Caveats.** Upgrade every server before sending `pinnedTo`: a 0.19 server refuses it as an unknown field. A 0.19
worker over HTTP keeps working against a 0.20 server (the server enforces pins) but cannot create pinned actors.

## 5. App guide: the command line and the terminal UI

**Where.** `docs/guide/app.md`, `Command line and terminal UI`.
**Text.** `cicada create actor vm/host-a --state '{}' --pinned-to host-a`; `cicada get actor vm/host-a -o wide` shows
`PINNED-TO`; `describe actor` and `-o json` show `pinnedTo`; the TUI's actor screen shows `Pinned to`. A bad
`--pinned-to` value is a usage error (exit code 2).
**Caveats.** No filter by pin; use the state listing and the mailbox to find and inspect pinned actors.

## 6. Operations: stable ids and waiting work

**Where.** `docs/operations.md`: `Deploying`, `On call` (a new subsection such as `Pinned actors that wait`),
`Node table`, `External leases`.
**Text.** Nodes and workers that own pinned actors need a stable id unique among live executors (`--node-id`,
`CICADA_NODE_ID`, `WorkerConfig::worker_id`); a random id after a restart leaves the old pins waiting. To diagnose
messages that never run: `cicada get actor <a> -o wide` (or `describe`) for `PINNED-TO`, `cicada get mailbox <a>` for
the backlog, `cicada get nodes` for whether a node with that id is up (a worker has no row). Fix by starting the owner,
or by deleting the actor and creating it again with another pin. Pause, purge, cancel and delete work from any node.
**Caveats.** Node rows and heartbeats are display only; a node shown alive does not prove that it polls the type.

## 7. Upgrading: drain old direct-store nodes for schema 8

**Where.** `docs/how-to/upgrading.md`: the introduction (it says stores write schema version 7; now 8) and a new
`**Version 0.20.0 is such a release.**` paragraph beside the other non-additive releases; `docs/operations.md`,
`Upgrading`, if it names the version.
**Text.** Migration 8 adds `actors.pinned_to` and the index `actors_pinned_due`; every existing actor is unpinned.
A 0.19 node that keeps running claims pinned actors whatever their pin, and a 0.19 binary refuses the migrated store:
stop every 0.19 node and worker that opens the store directly before the first 0.20 node starts; do not mix old and
new direct-store nodes. Upgrade HTTP servers before clients or workers send `pinnedTo`. Rust code: `ActorCreate`,
`ActorRecord`, `ActorSnapshot`, `CreateActorRequest`, `ActorCreateView` and `ActorView` have a new field; add
`pinned_to: None` to struct literals and exhaustive patterns.
**Caveats.** Never describe a mixed 0.19/0.20 direct-store fleet as safe.

## 8. Design guide: routing identity versus handles

**Where.** `docs/guide/design.md`: `Choose actors and identities`, `Work with outside systems`.
**Text.** A pin routes an actor's turns to one executor id; it does not store a live handle, authenticate a machine
or fence an outside effect after a lost lease. Pin an actor whose turns act on a machine-local resource; observe a
machine with a local task that sends messages instead. A live connection that can arrive at any node (a websocket)
fits a pin badly: keep the session-actor pattern of `A websocket session`.
**Caveats.** The executable process example follows in `pinned-process-example`; keep the existing anchors, which
the skill routes use.

## Blocking

None. `cargo test -p cicada-examples --doc`, `--test docs` and `--test skill` pass on this branch without prose
changes.

## Status

Ran on the branch: the core suite with PostgreSQL (1591 library tests and every integration suite), the placement
conformance cases on memory, SQLite and PostgreSQL, the upgrade tests on both stores, the fuzz comparison, the
`cicada-axum`, `cicada-client` and `cicada-app` (with `tui`) tests, the fixtures test, clippy with `-D warnings`, and
the docs, doctest and skill checks. All the text above is proposed; no protected prose is written. The generated
`docs/reference/cli.md` and `docs/openapi.json` are regenerated in the branch.
