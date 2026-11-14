# Docs handoff for store-close-cancellation

The docs session owns `README.md` and the hand-written docs under `docs/` until `build-loop-docs-handoff` lands. This
file holds the prose this change needs from it. The text below is complete, so the file stands alone.

What the code branch holds: the resumable close of `SqliteStore` (`crates/cicada/src/store/sqlite/store.rs`) and
`PostgresStore` (`crates/cicada/src/store/postgres/store.rs`, with the listener's stop split into `request_stop` and
`take_task` in `listener.rs`); the doc comments of `Store::close` and `Backend::close`; the tests in
`crates/cicada/src/store/sqlite/change_tests.rs` and `crates/cicada/src/tests/postgres/changes.rs`; the changelog entry
and version 0.17.4. No public signature, CLI, HTTP API, contract, schema or guarantee changed: `docs/reference/` and
`docs/guarantees.md` need nothing.

What users must learn, in one place: `Store::close` takes effect on its first poll (a never-polled future does
nothing); it returns only when the cleanup is done; a cancelled call leaves the store closed and the cleanup for the
next call, and concurrent calls all wait for the same end; nothing runs in the background meanwhile; there is no time
bound. PostgreSQL waits for borrowed main-pool connections, SQLite for its writer but not for reads in flight. A
connection whose own close was cancelled is left to the SQL driver. After the runtime shuts down, nothing is closed
gracefully. The memory store closes at once. The shutdown order of App and of embedded hosts does not change.

## 1. The close contract, in the architecture

**Where.** `docs/architecture.md`, `## Design notes`, a new bullet next to **PostgreSQL settings** (today around line
553). It is the reference for behaviour, so it holds the full contract.

**Agreed text.**

> - **Closing a store.** `Store::close` takes effect on its first poll: before it awaits anything, every call makes
>   operations unavailable, closes the hub and stops new connections (PostgreSQL marks its main pool closed and aborts
>   the listener; SQLite closes its reader permits). These steps are idempotent and each caller runs them, so a caller
>   never skips them because another set the closed flag first; a close future dropped unpolled has no effect. The
>   callers then take turns at a cleanup gate (a tokio mutex) that holds the progress, and only the last step marks it
>   done: PostgreSQL joins the aborted listener task, keeping its handle in the progress across a cancelled join, then
>   awaits the main pool's close, which waits for borrowed connections and closes the idle ones; SQLite waits for its
>   writer (a write or its settle task may hold it), closes it, then pops and closes the idle readers one at a time. A
>   cancelled caller leaves the connections it did not take for the next caller; one it had taken for its own close
>   drops into the driver's cleanup, which a later close does not join. No call returns only because the store is
>   closed, and no task finishes a cancelled close in the background: the next close, or the final drop of the store,
>   does. SQLite does not wait for reads in flight; a returning reader checks the closed flag under the pool's lock and
>   drops its connection, so it never refills a drained pool. There is no shutdown deadline, and after the runtime
>   shuts down what is left follows Tokio's and sqlx's drop rules, without a graceful close. The memory store closes
>   at once.

## 2. A cancelled or timed-out close, in operations

**Where.** `docs/operations.md`, `### Shutdown and probes`, after the sentence "Embedded hosts stop the node inside
axum’s shutdown future, then close the store or explicitly bound streams." (today around line 51).

**What users must learn.** A host may put a timeout on `Store::close` and call it again; the store stays closed in
between and the second call finishes the cleanup. A close can wait for a long time when an operation holds a
PostgreSQL connection or the SQLite writer.

**Agreed text.**

> `Store::close` has no time bound: on PostgreSQL it waits for connections that operations still hold, on SQLite for
> its writer (not for reads). A host may bound it with a timeout and call it again: the store stays closed, and the
> next call finishes the cleanup. A cancelled close leaves no background task, so call it again or drop the store.

## 3. Optional: one sentence in the core guide

**Where.** `docs/guide/core.md`, after the start-and-stop example, next to "The caller owns the store." (today around
line 201). One sentence is enough, and it may be left out if the docs session thinks operations covers it.

**Agreed text.**

> `store.close()` counts from its first poll and returns once the connections are released; if a timeout cancels it,
> the store stays closed and the next `close()` finishes the work.
