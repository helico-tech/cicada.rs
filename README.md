# Cicada for Rust — durable actors

A Rust port of [Cicada](https://github.com/helico-tech/cicada): a small durable actor system on a shared
transactional store (PostgreSQL or SQLite). An actor is a row of state plus a mailbox; it occupies memory only while
one of its messages is being handled. Every service that runs an `ActorSystem` is a node; nodes coordinate only through
the store.

The contract is the Kotlin implementation's: guarantees G1–G6, store requirements S1–S10, the store schema and the
fourteen-endpoint HTTP API. See [docs/architecture.md](docs/architecture.md).

Status: work in progress; see [work/queue.md](work/queue.md).
