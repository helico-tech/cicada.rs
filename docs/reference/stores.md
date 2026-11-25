# Store configuration


PostgreSQL (`PostgresOptions`, feature `postgres`):

| Setting | Default | Notes |
| --- | --- | --- |
| `schema` | `public` | Created on start when absent. Separate schemas let environments or tests share a database. |
| `retention` | 7 days | Outcome retention and the dedup window of message ids. |
| `max_pool_size` | 20 | At least `concurrency + 4`; see [Sizing](../operations.md#sizing). A node that hosts types warns at start, with both numbers and the settings to change, when its pool is smaller, and starts anyway. Each node checks only its own concurrency, not the sum of nodes or workers sharing one `Store`. |
| `socket_timeout` | 30 s | Longest single transaction attempt; then it fails and its connection is closed, so a dead peer without a TCP reset cannot block a claim, commit or stop. It also bounds the largest tree a delete or a stop can remove ([On call](../operations.md#on-call)). Zero disables it. |

Connection parameters (`sslmode`, `sslrootcert`, `application_name`, `options`, …) go in the URL of
`Store::postgres(url, options)`; it is the one way to open the store, and a setting a URL cannot carry is added to
`PostgresOptions` when a host needs it. For TLS see [TLS for PostgreSQL](#tls-for-postgresql). Store time
is the server's `clock_timestamp()`, read after each transaction's row locks, so run PostgreSQL with a disciplined
clock (NTP).

Change hints (ADR 0034) ride on PostgreSQL's `NOTIFY`. A transaction that changes an actor (a create, a new state, a
pause or resume, a removal) sends one notification of at most 7,998 bytes on the store's channel, with its wake-ups, in
one statement before `COMMIT`; a commit that changes no actor sends what it sent before. A commit that only writes a
state now notifies too: a little more work per commit, and one more entry in the server's notification queue (8 GB by
default; `max_notify_queue_pages` from PostgreSQL 17 on). The queue is cleared as every listener reads it, so a session
that listens on any channel and holds a transaction open, or stops reading, keeps it from being cleared;
`SELECT pg_notification_queue_usage()` shows how full it is. A full queue fails the `COMMIT` with SQLSTATE `54000`,
which the store reports as unavailable (`503`): the commit is not applied, and the usual rules for a commit that
fails apply (the message stays pending). A `COMMIT` can also time out or lose its answer, as before; the hints follow what the server committed. Every
store keeps one listening connection, started by the dispatch loop or by the first subscriber. Hints are best effort: a
host that needs every change of an older node, of SQL by hand, or after a runtime shut down keeps its own periodic full
read.

SQLite (`SqliteOptions`, feature `sqlite`):

| Setting | Default | Notes |
| --- | --- | --- |
| `retention` | 7 days | As for PostgreSQL. |
| `busy_timeout` | 5 s | How long a write waits for another process's lock before it fails as unavailable. |
| `clock` | system clock | The store clock; a virtual one in tests. |

The file runs in WAL mode with `synchronous = FULL`. Keep it on a local disk, not a network file system. Store time
never steps back (a persisted high-water mark), but a wall clock that jumps forward moves it forward too: leases
expire early and delayed messages fall due early, so run the host with NTP slewing.

### TLS for PostgreSQL

The feature `postgres` builds the driver with TLS (rustls); there is nothing else to switch on. Ask for TLS with
`sslmode` in the URL:

- `disable`, `allow`: plain text; the driver never tries TLS.
- `prefer` (the default): TLS without checking the server's certificate when the server offers TLS, plain text when
  it does not. Once the server has offered TLS, a failed handshake fails the connection; it does not go back to plain
  text (`sslmode=disable` does).
- `require`: TLS or no connection; the server's certificate is not checked.
- `verify-ca`: as `require`, and the certificate must be signed by a trusted CA.
- `verify-full`: as `verify-ca`, and the certificate must name the host of the URL.

The trusted CAs are the built-in list of public CAs (webpki-roots) and those in the PEM file that `sslrootcert` names
(`…?sslmode=verify-full&sslrootcert=/etc/ssl/db-ca.pem`); the certificate store of the operating system is not read.
A database rarely has a certificate of a public CA: one with a private or cloud-provider CA needs `sslrootcert`.
`sslcert` and `sslkey` name the files of a client certificate and its key, for a server that asks for one. For each of
the four that the URL does not name, the driver reads `PGSSLMODE`, `PGSSLROOTCERT`, `PGSSLCERT` or `PGSSLKEY` from the
environment, so the default mode can come from there. The default `prefer` neither insists on TLS nor checks the
server, so set the mode on purpose: `sslmode=verify-full`, or at least `require`.

The crates of Cicada build rustls on the crypto provider `aws-lc-rs`; a host that adds a library that builds rustls on
`ring` has two providers, and must install a default at start
(`rustls::crypto::aws_lc_rs::default_provider().install_default()`), or every call that leaves the choice to rustls
panics.

