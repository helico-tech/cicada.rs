# 0018. A smaller core, layers above it, and watches

Date: 2026-11-10

## Status

Accepted

## Context

A design review on 2026-11-10 asked what this library would look like if it had never been a port. The answer: the
kernel would be the same (a row plus a mailbox, lease and epoch fencing, one atomic commit, one-way sends, retry per
message, the store as the only clock), but less would sit around it. The core carries extension points with one user
(the public `Store` trait, the `Plugin` trait), a special case for actors without state, and structured keys. Getting
one actor to run behind HTTP takes about 150 lines of host code. There is no way for one actor to learn that another
one failed, except by polling outcomes.

The aim is the smallest robust core that still has opinions, a short way from an actor to a running node, and a small
set of ready-made actors on top. ADR 0008 allows free changes to the schema and the wire format while there is no
production database.

## Decision

### Layers

Dependencies point down only. Everything above the core uses its public API only; a higher crate that needs a private
hook shows that the core lacks a primitive.

| Layer | Crate | Contents |
| --- | --- | --- |
| 0 | `cicada` | Actor definition, runtime, the two SQL stores, watches, the read model |
| 1 | `cicada-axum`, `cicada-client` | The HTTP API with schema checks at the door; the typed client |
| 2 | `cicada-app` | Turns a set of actors into a runnable binary with a command line |
| 3 | `cicada-actors` | Ready-made actors: supervisor, dead letters, schedule and more |

### Kept

- Guarantees G1–G6 and the turn semantics, except where this ADR says otherwise.
- Explicit creation. A send to an absent instance still fails the commit. Lazy creation was considered and dropped:
  supervision needs a life cycle, and a mistyped key must fail loudly.
- Addresses of the form `type/key` (ADR 0013).
- `ActorRef` and `Actor` as two traits: the ref and its messages are what a sender depends on, the actor is the
  implementation.

### The core gets smaller

- **Every actor has state.** `Stateless`, the `ActorState` trait and `create_stateless` are removed. The core has an
  empty struct `NoState {}` that is stored as `{}`. State is never null in the store, on the wire or in a contract.
- **A key is one name.** A ref holds one name, not several fields joined with `:`. A singleton keeps the key `_`.
- **No namespaces.** There is no third part of an address. Dotted type names (`billing.invoices`) are the only
  grouping.
- **The store trait is not an extension point.** PostgreSQL and SQLite are the backends. Nobody outside this
  repository is expected to write one, so the trait leaves the public API.
- **One observer instead of plugins.** The `Plugin` trait and `Attributes` are removed. In their place is one narrow
  trait, `TurnObserver`: it is called when a turn starts, may add headers to the messages the turn sends, and returns
  a value that is told the outcome when the turn ends (dropped without that, the turn was abandoned). An observer
  cannot refuse a turn and cannot change what it commits. `cicada-otel` implements it and stays its own crate, so the
  core does not depend on OpenTelemetry. The core itself only writes `tracing` events.
- **One unhandled policy.** A message that cannot be decoded fails. `Unhandled::Drop` and the required `UNHANDLED`
  are removed.
- **Tidying.** The message type is declared once (on the ref). Constructors that return `Result<_, String>` return
  typed errors.

### JSON Schema is enforced at the HTTP door

A send over HTTP is checked against the registered JSON Schema of its message type, on every node, whether or not the
node hosts the type. A payload that does not match is refused with `400`. serde stays the judge at the turn. The
schemas stay discoverable through `GET /types/{name}`. The test harness checks that a type's samples pass both.

### Watches

A watch is a stored pair: a target and a watcher. The target is one actor, one type, or a dotted type prefix at a
segment boundary (`billing` matches `billing.invoices`). The watcher is one actor.

- A watch is set and removed as a turn effect (`watch`, `unwatch`), and from outside a turn through `ActorSystem` and
  HTTP.
- When a message to a watched actor ends as `failed`, or a watched actor stops or is deleted, the same transaction
  puts one notification message into each watcher's mailbox. The notification is a core message with a fixed type
  name and schema (`Failed` with message id, error and attempts, or `Stopped`).
- A watcher that does not exist is skipped and its watch removed. It never fails the commit of the watched actor.
- A watch survives delete and re-create of its target. Deleting a watcher removes its watches.
- A notification that itself fails notifies nobody, so two actors that watch each other cannot loop.
- A successful message notifies nobody.
- The watched actor does not know its watchers.

### Control effects

A turn can pause, resume, purge and delete another actor, committed atomically with its other effects. No effect
writes another actor's state. To restart an actor, a turn deletes it and creates it again; in one commit, deletes are
applied before creates.

### `cicada-app`

`App::new().host(actor).run()` gives a binary with `serve` (worker and HTTP), `worker` (a subset of types, no HTTP)
and the operator commands. The store and the listen address come from flags or environment variables. `App` only
composes layers 0 and 1; a host that needs more uses those directly. The operator commands always go over HTTP. The
unfinished `cicada-cli` becomes this crate's command line; the terminal UI is a feature that is off by default.

### `cicada-actors`

One crate, replacing `cicada-extras`. A ready-made actor has a fixed type name and stores the messages it sends for
others as an address, a message type name and a JSON payload; a typed helper builds that from a ref and a message, so
a wrong message is still a compile error. First: supervisor, dead letters, topic. Later, each when someone needs it:
schedule (moved from extras), debounce, rate limit, webhook delivery, circuit breaker, request with timeout,
scatter/gather. Sagas, step workflows, cursor sync, registries and singletons stay examples.

### Docs

One guide per layer in `docs/guide/`, with every Rust block compiled as a doctest and taken from tested examples.
Generated references (OpenAPI, command line help, the contracts of the ready-made actors) are checked for drift by a
test. A table maps each of G1–G6 and S1–S10 to the tests that prove it, and a test fails when a named test is gone.
`missing_docs` is an error.

### Order of work

Each step is one OpenSpec change that leaves the workspace green. The change is named after the colon, and its
proposal has an Order section that names the change before it:

1. Every actor has state: `every-actor-has-state`.
2. The store trait leaves the public API: `private-store-trait`.
3. A key is one name: `one-name-keys`.
4. Tidying: typed errors, one unhandled policy, the message type declared once: `tidy-the-core`.
5. Watches: `watches`.
6. Control effects: `control-effects`.
7. One observer instead of plugins: `turn-observer`.
8. JSON Schema at the HTTP door: `json-schema-at-the-http-door`.
9. `cicada-app`: `cicada-app`.
10. `cicada-actors`: supervisor, dead letters, topic: `cicada-actors`.
11. Docs: `docs-per-layer`.

Two changes outside this list have a place in it. `child-actors` (ADR 0019) is built between steps 4 and 5: watches
and control effects must know that removing an actor removes its children. `cli-and-tui` is archived before step 9,
which moves the command line into `cicada-app`.

## Left to the changes

- Whether `InMemoryStore` stays, or tests run on SQLite in memory with a virtual clock (step 2).
- Crash loops. `CrashLoopGuard` is a plugin, and a crash loop produces no `failed` outcome, so a watch does not see
  it. Step 7 needs an answer in the core, such as a claim limit that fails the message.
- What happens to a turn that runs on an actor while another turn deletes it (step 6).
- How a supervisor escalates: a failed turn commits nothing, so escalating is a committed turn that sends upward
  (step 10).
- A watch that covers only the messages its watcher sent, for request and reply on a shared worker. Not built until
  needed.

## Consequences

- Breaking for every user: refs, keys, state, contract hashes, the store schema and parts of the wire format change.
  Chorus and Karakuri must follow. Nodes before and after must not share a database.
- This supersedes parts of earlier ADRs: the three plugin hooks and `Unhandled` in 0012, the required `UNHANDLED` in 0003, and keys
  of several fields in 0013.
- A failed message can now cause sends (notifications). G2 and G4 get a row for that in `docs/architecture.md` when
  step 5 lands.
- Supervision is opt-in and lives outside the core. An actor nobody watches behaves as today.
- Tenancy across types has no support. Adding a namespace later means changing the address format again.
- The operator surface of ADR 0014 and ADR 0015 stays; it moves behind `cicada-app` for hosts that want it ready-made.
