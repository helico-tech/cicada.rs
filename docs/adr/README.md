# Decisions

The decisions of Cicada for Rust, one ADR per file (Nygard format: status, context, decision, consequences). An
accepted ADR is not rewritten: a later one supersedes it. A change that makes a decision adds its ADR's row here.

| ADR | Decision |
| --- | --- |
| [0001](0001-port-keeps-the-contract.md) | Superseded by 0008. |
| [0002](0002-crate-stack.md) | Crate stack: tokio, axum 0.8, sqlx 0.9 (one store per backend), serde, schemars, chrono, thiserror/anyhow, async-trait, reqwest, tracing and opentelemetry. |
| [0003](0003-actor-definition-api.md) | Actor definitions: a ref struct implementing `ActorRef`, a tagged serde enum, and the `Actor` trait with one `handle`; no proc macro. Its unhandled policy is gone (0018). |
| [0004](0004-shared-store-schema.md) | Superseded by 0008. |
| [0005](0005-reuse-the-console.md) | Superseded by 0011. |
| [0006](0006-console-in-sensus-design.md) | Superseded by 0011. |
| [0007](0007-container-handles.md) | Superseded by 0011. |
| [0008](0008-rust-only.md) | Cicada for Rust stands alone: no contract with the Kotlin implementation. |
| [0009](0009-one-baseline-schema.md) | The six early migrations are merged into one schema at version 1. |
| [0010](0010-openspec-for-work-tracking.md) | Changes, designs and tasks live in `openspec/` (OpenSpec). |
| [0011](0011-library-only.md) | A library only: the console (`cicada-ui`) and container handles (`cicada-docker`) are removed. |
| [0012](0012-slimmer-api-and-wire-format.md) | A slimmer API and wire format: turn hooks that only observe (replaced by one observer in 0018), one way to configure and send, ISO-8601 durations only, twelve endpoints. |
| [0013](0013-type-and-key-addresses.md) | Actors are addressed by type name and key (`sync/acme`) instead of URIs. |
| [0014](0014-management-reads-and-type-removal.md) | Management reads over HTTP (outcomes, stats, caused-by) and removing a type's registration. |
| [0015](0015-operator-actions.md) | Operator actions: node table, cancelling a running turn, pausing and purging a mailbox, retrying a message, bulk actions; the store refuses creates of an unregistered type. |
| [0016](0016-releases-are-tags.md) | A release is a tag on `main` with one version for the workspace; a release passes all checks first and never moves. |
| [0017](0017-contracts-ignore-descriptions-and-stores-refuse-foreign-databases.md) | A contract hash leaves out the schemas' descriptions (the published contract keeps them); the SQLite store refuses a database that holds other tables. |
| [0018](0018-a-smaller-core-layers-and-watches.md) | A smaller core (every actor has state, one-name keys, one turn observer in place of three hooks, no public store trait), layers above it (`cicada-app`, `cicada-actors`), JSON Schema checks at the HTTP door, watches and control effects. |
| [0019](0019-child-actors.md) | Child actors: an actor may have a parent, the actor whose turn created it, and removing the parent removes what it owns. |
| [0020](0020-the-change-loop.md) | Superseded by 0030. |
| [0021](0021-a-key-is-one-name.md) | A key is one name: a ref is written as one string or a unit, a `:` is an ordinary character, contracts hold no key fields, and a database is upgraded in place. |
| [0022](0022-typed-errors-and-no-such-type.md) | Typed errors: a named error per module, a start error with a problem per cause (a contract conflict is its own case), a send to an unregistered type answers `404 no-such-type`, and a migration that violates a constraint is an invalid store. |
| [0023](0023-the-rules-of-a-watch.md) | The rules of a watch: notifications in the watcher's own family, a type target at a dot and `*` for every type (failures only), `Stopped` with its cause and `by`, a notification marked and named after its event, and G2 and G4 that name the notifications. |
| [0024](0024-control-effects.md) | Control effects: a turn pauses, resumes, purges and deletes another actor with the meaning of the operator's calls, an effect on an actor that is not there does nothing, a turn cannot delete itself or an ancestor, deletes come before creates (a restart), and G1, G2 and S7 that say so. |
| [0025](0025-the-claim-limit.md) | The claim limit: a message counts its lost claims (a claim that another claim replaced although its turn neither committed nor gave the lease back), a node fails a message with as many as its limit (5 by default) through a commit that tells its watchers, and G4 that names the limit. |
| [0026](0026-the-schema-check-at-the-http-door.md) | The schema check at the HTTP door (narrowed by 0029): a send and a create over HTTP are checked against the registered schema on every node and refused with `422` (not `400` as 0018 said), a hosting node still decodes, a repeated send is answered first, no format is asserted, a schema the validator cannot use is not enforced, and JSON that code sends is not checked. |
| [0027](0027-the-app-layer.md) | The app layer: `serve` takes `--types` too, a binary without types has neither `serve` nor `worker`, `App` has no authentication of its own and `serve` listens on `127.0.0.1:8080` unless told otherwise, the node stops before the HTTP server, every setting has a flag and a `CICADA_…` variable, and `cicada-cli` goes into `cicada-app`, with the terminal UI and each store as features that are off by default. |
| [0028](0028-the-ready-made-actors.md) | The ready-made actors: a `Failed` holds its cause, a turn creates an actor by its address, a supervisor restarts a child at fault once per incident by the order of its own mailbox and escalates by failing a message of its own (as a handler for too many restarts, refused for good for a child it cannot start), a stored message holds no address, and a topic delivers to each subscriber in a turn of its own. |
| [0029](0029-the-depth-of-a-value.md) | The depth of a value: a payload and a state hold at most 64 levels, refused like the other limits of a value (before the repeated id and the schema check over HTTP, at the call in a turn, as a failed turn for a new state, and at an operator's retry, `409`); values stored before are read as before; a `Failed` can be one level deeper. |
| [0030](0030-three-loops.md) | Three loops, each in a session of its own: design with the owner, build in a worktree, and review in any agent harness, handing over through files (`scripts/status.py`, `scripts/review.py`); changes are small, the builder runs targeted tests, reviews read rather than run the suite, and only blocking findings start one fix round. |
| [0031](0031-durable-flows.md) | Host-named flow actors replay an ordered journal, commit one new step per turn, recover through an armed continuation and retain completed runs; external effects remain at least once, and the first journal contract includes durable sleep. |
| [0032](0032-timer-trace-links.md) | Observers select headers for delayed self-sends or other sends; cicada-otel starts timer traces with a link carried by prefixed propagation headers, preserving explicit parents and ordinary send propagation. |
| [0033](0033-listing-actors-with-state.md) | A typed actor-state listing and GET /actors/states reuse full actor snapshots, with one set-based read per page and ordinary address keyset paging. |
| [0034](0034-actor-change-hints.md) | Best-effort actor change hints with bounded resync recovery, shared-Store memory/SQLite coverage and transactional PostgreSQL notifications across upgraded nodes. |

## Design questions

| Question | Decision |
| --- | --- |
| Crash loops | The claim limit in the core, on by default (ADR 0025): a message with 5 lost claims ends `failed`; `lostClaims` is on the envelope and in `/messages/{id}`. |
| Contract changes | Strict contract-hash equality; a node with a different contract for a registered type name fails to start. Contracts include the generated schemas, so a schemars upgrade can change hashes. Doc comments (`description`) do not count in the hash. |
| State evolution | No migrations; changes must be additive with defaults (`#[serde(default)]`). |
| Observer data | `headers` on the envelope, which an observer may add to a turn's sends; `correlation_id` is kept (the HTTP contract uses it). |
| Send to an absent instance inside a turn | The whole turn fails at commit: outcome `failed` with the reason, no effects. |
| Admin delete | Allowed: `DELETE /actors/{type}/{key}`. |
| One-way messaging | Kept; no ask/await. |
| Defaults | Lease 30 s, heartbeat 10 s, poll 500 ms, max turn 15 min, retention 7 days; all configurable. |
