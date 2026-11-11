# Design

Described with the owner on 2026-11-11 (change loop, Describe). The owner chose a lighter loop for this change: no
design review, but a builder and a validator as usual (decision 5).

## Context

- The workspace names the driver as `sqlx = { version = "0.9.0", default-features = false, features =
  ["runtime-tokio", "json", "chrono"] }`. sqlx 0.9.0 compiles TLS in only with one of its TLS features:
  `tls-rustls-ring-webpki` (also named `tls-rustls`), `tls-rustls-ring-native-roots`, `tls-rustls-aws-lc-rs` or
  `tls-native-tls`. Without one, a URL that asks for TLS fails with "SQLx was built without TLS support enabled".
- The `postgres` feature of `cicada` is `["dep:sqlx", "sqlx/postgres"]`; the `sqlite` feature is
  `["dep:sqlx", "sqlx/sqlite"]`.
- The driver reads these parameters from a PostgreSQL URL: `sslmode` (`disable`, `allow`, `prefer`, `require`,
  `verify-ca`, `verify-full`), `sslrootcert`, `sslcert` and `sslkey`. Its default mode is `prefer`: try TLS first,
  and go on without it when that fails.
- rustls is in the workspace already, through the HTTP client of `cicada-client` and `cicada-cli`.
- The tests and CI run one PostgreSQL without TLS, on port 55432 (`CICADA_PG_URL`).

## Goals / Non-Goals

Goals:

- A host that names the `postgres` feature can open a store over TLS, with nothing else to switch on.

Non-Goals:

- Testing the driver's TLS: which modes verify what is the driver's business.
- A PostgreSQL with TLS in Docker or in CI.

## Decisions

### 1. rustls

The driver is built with rustls: pure Rust, no system library to build against, and in the workspace already.

Not chosen: the TLS library of the operating system (build images would need its headers).

### 2. The certificate store of the operating system

When a URL asks to verify the server, the certificates that the operating system trusts are trusted. A database
rarely has a certificate of a public CA, so operations install their CA on the host, or name a CA file with
`sslrootcert` in the URL. The driver feature is `tls-rustls-ring-native-roots`.

Not chosen: the built-in list of public CAs (a private or cloud-provider CA would always need a CA file).

### 3. TLS comes with the `postgres` feature

`postgres` becomes `["dep:sqlx", "sqlx/postgres", "sqlx/tls-rustls-ring-native-roots"]`. The `sqlite` feature stays
as it is, so a SQLite host builds no TLS for its store.

Not chosen: a feature of its own (a host that forgets it would get today's error at run time).

### 4. One test that TLS is compiled in

With `CICADA_PG_URL` set, a store is opened on that URL with `sslmode=require`. The test database offers no TLS, so
the open must fail, and the error must be the one for a server without TLS, not the one for a build without it. The
test is skipped without the URL, like the other PostgreSQL tests.

Not chosen: a second PostgreSQL with TLS in Docker and CI, with tests for each mode (it would test the driver).

### 5. A lighter loop

One line in a manifest, one test and a paragraph of docs. There is no design review; a builder builds it and a
validator checks it. It is released when it lands, as 0.5.1.

### 6. What the docs say

Decided by the main session.

`docs/operations.md` gets a paragraph on TLS for PostgreSQL: the modes of `sslmode` in one line each, `sslrootcert`
for a CA file, and that the default `prefer` uses TLS without verifying when the server offers it and plain text
otherwise, so an operator who wants TLS sets `sslmode=verify-full`, or at least `require`. The README names the
paragraph where it shows how to open PostgreSQL.

## Risks / Trade-offs

- **A URL without `sslmode` now uses TLS when the server offers it**, without verifying the server. → It was plain
  text before, so nothing gets weaker. The docs say to set the mode on purpose.
- **A PostgreSQL host builds more crates.** → rustls and its parts; most are in the workspace already.
- **The test proves that TLS is compiled in, not that a TLS connection works.** → Accepted (decision 4).

## Migration Plan

None. A host changes nothing; a URL that asks for TLS starts to work.
