# Architecture

This document is the reference for Cicada's domain language, guarantees G1–G6, store requirements S1–S10 and HTTP
contract. Decisions are recorded in the ADRs in [adr/](adr/).

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
| Actor type | A definition: a ref type with its type name, a state type, a message family, a handler, an unhandled policy. In Rust: an implementation of the `Actor` trait. A type that keeps no data has the state `NoState`, stored as `{}`. |
| Contract | The published description of a type: type name, message type names, and the JSON Schemas generated from its types, the state's always among them. It holds nothing of the ref (ADR 0021). Its hash leaves out the schemas' `description`s (doc comments), so a reworded comment keeps the hash (ADR 0017). |
| Actor instance | One name of a type, explicitly created with its initial state. Owns one state value, never absent and never JSON `null`, and one mailbox (ADR 0018). |
| Ref | An actor's address: a type name and a key, `type/key`. A typed ref in code (`ActorRef`); its address text (`sync/acme`) on the wire, in state and in messages. The key is one name (ADR 0021): serde writes the ref as one string (a struct with one string field, or a type written as a string itself), or the ref is a unit struct or a struct that declares no field to serde, a singleton with the key `_`. A name is any non-empty text; in the key it keeps `A-Z a-z 0-9 . _ ~ -` and `:` and percent-encodes the rest, so a `:` separates nothing. |
| Message | A typed envelope addressed to one instance, with a delivery time. |
| Mailbox | An instance's pending messages, ordered by delivery time. |
| Turn | One handling of one message: claim, load, run handler, commit. |
| Effects | What a handler produces: new state, sends, creates, cancels, stop. Buffered until commit. |
| Commit | The single atomic write that applies a turn's effects and completes its message. |
| Lease | A node's exclusive, time-limited right to run a turn for one instance. |
| Epoch | Per-instance counter, increased on every claim. The fencing token for commits. |
| Outcome | The terminal result of a message: `done`, `failed`, `cancelled` or `purged`. |
| Plugin | An optional extension that observes a turn through three hooks and may refuse it at claim. It never changes what a turn commits, except that it may add headers to the turn's sends. The core uses none. |

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

The backends are PostgreSQL and SQLite, each a cargo feature of `cicada` (`postgres`, `sqlite`), and the in-memory
store of the tests (feature `testing`). A host opens one as the public type `Store` (`Store::postgres`,
`Store::sqlite`, `Store::memory`) and reads what it holds through `ActorSystem`; the backend trait is private to
`cicada`, so a store is not an extension point (ADR 0018). Every backend passes the conformance suite, test code of
`cicada` (`crates/cicada/src/tests/conformance`).

| # | Guarantee |
| --- | --- |
| S1 Durability | An acknowledged write survives process and machine crash. |
| S2 Atomicity | `commit` applies all of its parts or none. `delete` removes instance and mailbox together. |
| S3 Per-instance linearizability | All operations on one instance and its mailbox have a single order. A read after an acknowledged write sees it. |
| S4 Lease exclusivity | At most one unexpired lease per instance. Each claim issues an epoch greater than any earlier one for that address, also across delete and re-create. |
| S5 Fencing | `commit` and `renew` succeed only with the current epoch. |
| S6 Order | `claim_due` returns, per instance, only the pending message with the smallest `(deliverAt, seq)`, and only when it is due. |
| S7 No loss, no duplicate | A message leaves pending only by commit, cancel or delete. Message ids are unique across pending messages and retained outcomes. |
| S8 Liveness | An instance that is not paused, with a due head message and no valid lease, is eventually returned by `claim_due`. A paused instance is not claimed until it is resumed (ADR 0015). |
| S9 One clock | Due-ness and lease expiry use the store clock only. |
| S10 Retention | Outcomes are kept for a configured period, which is also the dedup window. A message's retention policy can hide its outcome earlier or keep it longer; a minimal record stays for the whole dedup window. |

The SQL schema (tables `actor_types`, `actors`, `messages`, `outcomes`, plus sequences) is versioned and migrated
when a store opens a database; it is at version 3. `actors.state` is required in both stores, so a store refuses an
instance without a state. Migration 2 gave every instance without a state the state `{}` and removed the registrations
without a state schema (types that were stateless before ADR 0018). Migration 3 removed every registration, because
key fields left every contract, and rewrote `%3A` to `:` in `actors.address`, `messages.actor_address` and
`outcomes.actor_address` (ADR 0021); two old addresses that become one stop it. Unlike the others these two are not
additive. The SQLite
store sets up only a new or empty database: one without a schema version that already holds tables belongs to another
program, and opening it fails and leaves it as it was (ADR 0017).

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
| Handler returns, but its new state cannot be encoded or is JSON `null` | `failed`, error recorded, never retried | None |
| Commit rejected: a send targets an absent instance | `failed`, reason recorded | None |
| Turn exceeds its maximum duration and is cut off | As a failing handler: retried by the policy, else `failed` (`timeout`) | None |
| Handler returned, but only after its maximum duration (blocking code) | `failed` (`timeout`), never retried | None |
| Node dies or loses the lease before commit | None; message stays pending | None |
| Instance stopped or deleted | `purged` for all still-pending messages | — |
| Admin cancels a pending message | `cancelled` | — |
| Admin cancels a running turn, before its handler ended | `cancelled` at the owning node's next heartbeat, no retry; the mailbox moves on. A handler stops at its next `.await`; blocking code runs until it returns, and the commit is then `cancelled` rather than done (`on_outcome` sees it, `on_handler_error` does not run). A request after the handler ended is ignored (G6) | None |
| Admin purges a mailbox | `purged` for every pending message except one being handled | — |

### How the runtime maps to tokio

Each claimed turn is a task with three children: *work* (load, `on_claim`, the handler under `tokio::time::timeout`,
checks), *keeper* (heartbeat at a third of the TTL, plus a watchdog on the local lease deadline; it aborts
*work* when the lease is lost) and *overtime* (warns about handlers that do not yield). The turn task awaits *work* and
then commits while the keeper keeps renewing. Cancellation is tokio's: an aborted future stops at its next `.await`, so
a handler blocked in synchronous code keeps its turn until it returns. Blocking work belongs in `spawn_blocking`. A cancelled turn (node stop after the grace period,
or a dropped `run_due_turns` future) aborts *work*, waits for it to stop, and releases its lease only if the handler
had not ended. A claim the store answers after its caller is gone is released, never left to expire.

An admin cancel of a running turn is recorded by the store as the turn's epoch and read by the keeper at its next renewal, so it takes up to one heartbeat. The keeper aborts *work*, waits for it to stop and commits `cancelled` itself, fenced by its epoch, so G1 and G3 hold and no other node runs the message meanwhile. Whether the cancel request or the handler's end came first is decided once, by one atomic flag: a request that lost is ignored and the turn commits normally. No request can reach a later turn, because each stores the epoch it was for. A lease is never taken away from a running node: there is no operation that does so (ADR 0015).

All local timing uses `tokio::time`, so tests on a paused runtime (`start_paused = true`) run on virtual time: the
in-memory store reads its clock from `tokio::time::Instant` (`TokioClock`).

## Definition API

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sync { tenant: String }

impl ActorRef for Sync {
    const TYPE: &'static str = "sync";
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
    const UNHANDLED: Unhandled = Unhandled::Fail;   // mandatory: Fail | Drop

    async fn handle(&self, turn: &mut Turn<Self>, msg: SyncMsg) -> HandlerResult {
        match msg {
            SyncMsg::Run => { turn.state_mut().imported = 0; turn.send_self(SyncMsg::FetchPage { cursor: None })?; }
            SyncMsg::FetchPage { cursor } => { /* … */ }
        }
        Ok(())
    }
}
```

## Crates

| Crate | Contents |
| --- | --- |
| `cicada` | Addressing, definition API, runtime and dispatch loop, the store (`Store` and its model; the backend trait is private), contracts and schemas, retry and retention policies, the HTTP wire model (`api`, shared by server and client). Behind features: the SQLite store (`sqlite`; sqlx, one file, WAL, one writer), the PostgreSQL store (`postgres`; sqlx, `FOR UPDATE SKIP LOCKED` claims, `LISTEN/NOTIFY` wake-ups) and the test support (`testing`: the in-memory store, clocks, `TestActors`, `claim_and_abandon`). Its tests hold the S1–S10 conformance suite every backend runs and the randomised tests: parsers, backend comparison, and a node simulation under injected faults |
| `cicada-test` | Exports the test support of `cicada` (`TestActors`, clocks, `claim_and_abandon`); contract snapshots and state samples |
| `cicada-axum` | `actor_api()`, `actor_api_read()` and `actor_health()` as axum routers, and the OpenAPI document |
| `cicada-client` | Typed HTTP client (reqwest) |
| `cicada-cli` | The binary `cicada`: commands and a terminal UI over the HTTP API, through `cicada-client`; no library crate depends on it |
| `cicada-otel` | OpenTelemetry plugin |
| `cicada-extras` | Schedules (interval, daily), crash-loop guard, failure watcher, dead letters |
| `examples` | Worked examples with tests |
| `sample-service` | A runnable node (binary) with the examples, used by end-to-end tests |
| `e2e` (`cicada-e2e`) | End-to-end and fault tests: sample nodes in Docker on PostgreSQL and SQLite, driven over HTTP (`CICADA_E2E=1`) |

## HTTP contract

See the README for the endpoints and `docs/openapi.json` for the OpenAPI document. Durations are ISO-8601 strings (`PT5S`); instants are ISO-8601 in UTC. Errors are `application/problem+json` (RFC 9457) with `type` `urn:cicada:problem:<slug>`. The routes are an
axum `Router` that the host nests into its own router and wraps in its own authentication layers. The bodies are the
types of `cicada::api`, used by both `cicada-axum` and `cicada-client`; request envelopes decode strictly (an unknown
key is `400`), payloads and states leniently.
