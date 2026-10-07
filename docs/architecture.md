# Architecture

Cicada for Rust is a port of [Cicada for Kotlin](https://github.com/helico-tech/cicada) (`ARCHITECTURE.md` there is
the original design). It keeps the contract: the domain language, guarantees G1–G6, store requirements S1–S10, the
store schema and the HTTP API. What changed is how the contract is expressed in Rust; see the ADRs in
[adr/](adr/).

Priorities, in order: simplicity, ease of use, correctness. Throughput is not a goal.

## Scope

A library embedded in Rust services. Every service that runs an `ActorSystem` is a **node**. Nodes are peers that
coordinate only through one shared transactional store (PostgreSQL, or SQLite for one host). An actor is a row of
state plus a mailbox; it occupies memory only while one message is being handled.

| Decision | Choice | Consequence |
| --- | --- | --- |
| Topology | Embedded peer nodes, one shared store | No coordinator and no cluster protocol. |
| Runtime | tokio; the HTTP API is an axum `Router` | Services that host actors are tokio services. axum supplies HTTP; the host supplies authentication. |
| Long work | Lease + heartbeat | A handler may run for minutes. A turn whose node dies is re-run elsewhere. |
| Retries | Per message, at dispatch | The sender attaches a retry policy; a failed run puts the same message back until the policy is exhausted. |
| Validation | The serializer (serde) is the only validator | JSON Schemas are generated (schemars) and published, never used to validate. |
| Messaging | One-way send only | A reply is an ordinary send to the sender's address. |
| Mailbox + state | One transaction boundary | State, outgoing messages and message completion commit atomically. |

## Domain language

| Term | Meaning |
| --- | --- |
| Node | A service running an `ActorSystem`. Sends messages, serves the HTTP API, and runs turns for the actor types it hosts. |
| Store | The shared durable backend: mailbox, state store, type registry, the atomic commit, and the only clock. |
| Actor type | A definition: a ref type with its URI template, an optional state type, a message family, a handler, an unhandled policy. In Rust: an implementation of the `Actor` trait. |
| Contract | The published description of a type: template, message type names, and the JSON Schemas generated from its types. |
| Actor instance | One concrete URI of a type, explicitly created. Owns one state value and one mailbox. |
| Ref | An actor's address. A typed ref struct in code (`ActorRef`); its URI string on the wire, in state and in messages. |
| Message | A typed envelope addressed to one instance, with a delivery time. |
| Mailbox | An instance's pending messages, ordered by delivery time. |
| Turn | One handling of one message: claim, load, run handler, commit. |
| Effects | What a handler produces: new state, sends, creates, cancels, stop. Buffered until commit. |
| Commit | The single atomic write that applies a turn's effects and completes its message. |
| Lease | A node's exclusive, time-limited right to run a turn for one instance. |
| Epoch | Per-instance counter, increased on every claim. The fencing token for commits. |
| Outcome | The terminal result of a message: `done`, `failed`, `cancelled` or `purged`. |
| Plugin | An optional extension that observes or alters a turn through four hooks. The core uses none. |

## Guarantees

| # | Guarantee |
| --- | --- |
| G1 Serial | Turns of one instance never overlap, and at most one turn commits per epoch. |
| G2 Atomic | State change, creates, sends and message completion happen all together or not at all. |
| G3 One outcome | Every accepted message reaches exactly one outcome. None is lost; none completes twice. |
| G4 Effects once | Effects inside the system are exactly-once. Handler execution and external side effects are at-least-once, and repeat only after a node dies mid-turn. |
| G5 Order | Per instance, delivery follows `(deliverAt, seq)` and never happens early. |
| G6 No hidden retry | The runtime never re-runs a handler that returned. It re-runs a handler that failed only when the message's own retry policy asks for it. |

A handler that calls an external system passes `turn.message().id` as idempotency key; `attempt > 1` tells it that an
earlier run may have got part-way.

## Store requirements

Every backend implements the `cicada::store::Store` trait and passes the conformance suite in
`cicada-store-conformance`.

| # | Guarantee |
| --- | --- |
| S1 Durability | An acknowledged write survives process and machine crash. |
| S2 Atomicity | `commit` applies all of its parts or none. `delete` removes instance and mailbox together. |
| S3 Per-instance linearizability | All operations on one instance and its mailbox have a single order. A read after an acknowledged write sees it. |
| S4 Lease exclusivity | At most one unexpired lease per instance. Each claim issues an epoch greater than any earlier one for that URI, also across delete and re-create. |
| S5 Fencing | `commit` and `renew` succeed only with the current epoch. |
| S6 Order | `claim_due` returns, per instance, only the pending message with the smallest `(deliverAt, seq)`, and only when it is due. |
| S7 No loss, no duplicate | A message leaves pending only by commit, cancel or delete. Message ids are unique across pending messages and retained outcomes. |
| S8 Liveness | An instance with a due head message and no valid lease is eventually returned by `claim_due`. |
| S9 One clock | Due-ness and lease expiry use the store clock only. |
| S10 Retention | Outcomes are kept for a configured period, which is also the dedup window. A message's retention policy can hide its outcome earlier or keep it longer; a minimal record stays for the whole dedup window. |

The SQL schema (tables `actor_types`, `actors`, `messages`, `outcomes`, plus sequences) is the same as the Kotlin
implementation's, migrations included, so both implementations read each other's databases (ADR 0004).

## Turn semantics

1. **Claim.** A node atomically takes the lease of an instance that has a due head message and no valid lease. Epoch
   and `attempt` each increase by 1.
2. **Load.** State, the own ref and the message are decoded. If the payload cannot be decoded, the unhandled policy
   runs instead of the handler.
3. **Run.** The handler runs as a future. Effects are buffered in the `Turn`. The node renews the lease by heartbeat.
4. **Commit.** One write, fenced by the epoch: outcome (or reschedule), cancels, creates, sends, state, lease release.

| Situation | Message outcome | Effects |
| --- | --- | --- |
| Handler returns `Ok` | `done` | Applied atomically |
| Handler returns `Err` or panics, retry policy with attempts left, failure not permanent | None; the same message returns to pending after the backoff, `failures` + 1, `lastError` set | None |
| Handler fails (no retry left, or permanent), or state cannot be decoded | `failed`, error recorded | None |
| Commit rejected: a send targets an absent instance | `failed`, reason recorded | None |
| Turn exceeds its maximum duration and is cut off | As a failing handler: retried by the policy, else `failed` (`timeout`) | None |
| Handler returned, but only after its maximum duration (blocking code) | `failed` (`timeout`), never retried | None |
| Node dies or loses the lease before commit | None; message stays pending | None |
| Instance stopped or deleted | `purged` for all still-pending messages | — |
| Admin cancels a pending message | `cancelled` | — |

### How the runtime maps to tokio

Each claimed turn is a task with three children: *work* (load, `on_claim`, the handler under `tokio::time::timeout`,
hooks, checks), *keeper* (heartbeat at a third of the TTL, plus a watchdog on the local lease deadline; it aborts
*work* when the lease is lost) and *overtime* (warns about handlers that do not yield). The turn task awaits *work* and
then commits while the keeper keeps renewing. Cancellation is tokio's: an aborted future stops at its next `.await`, so
a handler blocked in synchronous code keeps its turn until it returns, exactly like a Kotlin handler that ignores
coroutine cancellation. Blocking work belongs in `spawn_blocking`.

All local timing uses `tokio::time`, so tests on a paused runtime (`start_paused = true`) run on virtual time: the
in-memory store reads its clock from `tokio::time::Instant` (`TokioClock`).

## Definition API

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sync { tenant: String }

impl ActorRef for Sync {
    const TEMPLATE: &'static str = "actor://sync/{tenant}";
    type Message = SyncMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SyncMsg { Run, FetchPage { cursor: Option<String> } }

struct SyncActor { api: ApiClient }

impl Actor for SyncActor {
    type Ref = Sync;
    type State = SyncState;
    type Message = SyncMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;   // mandatory: Fail | Drop | Custom

    async fn handle(&self, turn: &mut Turn<Self>, msg: SyncMsg) -> HandlerResult {
        match msg {
            SyncMsg::Run => { turn.state_mut().imported = 0; turn.send_self(SyncMsg::FetchPage { cursor: None })?; }
            SyncMsg::FetchPage { cursor } => { /* … */ }
        }
        Ok(())
    }
}
```

| Kotlin | Rust |
| --- | --- |
| `@ActorUri` ref data class | struct implementing `ActorRef` (`TEMPLATE`, `Message`); fields ↔ template params via serde |
| sealed message family, `@SerialName` | enum with `#[serde(tag = "type")]`; variant name (or `#[serde(rename)]`) is the type name |
| `on<T> { }` per message, checked at node start | one `handle` with an exhaustive `match`, checked by the compiler |
| `unhandled { fail() }` mandatory | `const UNHANDLED: Unhandled` without default: the compiler requires it |
| `TurnScope` (`self`, `state`, `send`, `create`, `stop`, `cancel`, `fail`) | `Turn<Self>` (`self_ref`, `state`/`state_mut`, `send`/`send_with`/`send_self`/`send_to`/`send_raw`, `create`, `stop`, `cancel`, `fail`) |
| exceptions, `PermanentFailure` | `Result<(), HandlerError>`; `?` for ordinary failures, `HandlerError::permanent` / `turn.fail(..)` for the veto; panics are caught as failures |
| plugin DSL with four hooks | `Plugin` trait with four async hooks |
| `Attributes` with `AttributeKey<T>` | `Attributes` keyed by type (like `http::Extensions`) |
| `testActors { }` on virtual time | `TestActors` on tokio's paused clock |

## Crates

| Crate | Contents | Kotlin module |
| --- | --- | --- |
| `cicada` | Addressing, definition API, runtime and dispatch loop, store trait and model, contracts and schemas, retry and retention policies | `actors-core` |
| `cicada-test` | `InMemoryStore`, clocks, `TestActors`, contract snapshots and state samples | `actors-test` |
| `cicada-store-conformance` | The S1–S10 suite every store runs | `actors-store-conformance` |
| `cicada-store-sqlite` | `SqliteStore` (sqlx): one file, WAL, one writer | `actors-store-sqlite` |
| `cicada-store-postgres` | `PostgresStore` (sqlx): `FOR UPDATE SKIP LOCKED` claims, `LISTEN/NOTIFY` wake-ups | `actors-store-postgres` |
| `cicada-axum` | `actor_api()` (the fourteen endpoints) and `actor_health()` as axum routers; the HTTP wire model | `actors-ktor` |
| `cicada-client` | Typed HTTP client (reqwest) | `actors-client` |
| `cicada-ui` | The actor console (the React app of the Kotlin repo), embedded and served by `actor_ui()` | `actors-ui` |
| `cicada-otel` | OpenTelemetry plugin | `actors-otel` |
| `cicada-extras` | Timer, schedules (interval, daily), crash-loop guard, failure watcher, dead letters | `actors-extras` |
| `examples` | Worked examples with tests | `examples` |
| `sample-service` | A runnable node (binary) with the examples, used by end-to-end tests | `sample-service` |

## HTTP contract

Fourteen endpoints, identical to the Kotlin implementation (paths, bodies, status codes, problem types); see the
README. Errors are `application/problem+json` (RFC 9457) with `type` `urn:cicada:problem:<slug>`. The routes are an
axum `Router` that the host nests into its own router and wraps in its own authentication layers.
