# Proposal

## Why

A node cannot reach PostgreSQL over TLS. The workspace names the driver without a TLS feature
(`sqlx = { version = "0.9.0", default-features = false, features = ["runtime-tokio", "json", "chrono"] }`), and the
driver compiles TLS in only when one is named. So `Store::postgres("postgres://…?sslmode=require", …)` fails with
"SQLx was built without TLS support enabled". The validator of `private-store-trait` found it on 2026-11-11; it is the
same in every release so far. Since 0.5.0 a URL is the only way to open PostgreSQL, and the driver is a private
dependency of `cicada`, so a host has no clean way to switch TLS on itself.

## What Changes

- The `postgres` feature of `cicada` builds the driver with TLS: rustls on `aws-lc-rs`, the crypto provider of the
  HTTP client, trusting the built-in list of public CAs. A URL with `sslmode=require`, `verify-ca` or `verify-full`
  then works, and `sslrootcert`, `sslcert` and `sslkey` in the URL name files, as the driver reads them; a private CA
  is named with `sslrootcert`.
- One test shows that TLS is compiled in: a connection with `sslmode=require` to the plain test database fails
  because the server offers no TLS, not because the driver has none.
- CI fails when the `sqlite` feature builds a TLS library, and when the workspace builds rustls with two crypto
  providers (`ring` and `aws-lc-rs`): then every caller that leaves the choice of provider to rustls panics.
- `docs/operations.md` says how to ask for TLS in the URL or with `PGSSLMODE` and its siblings; which CAs are trusted;
  that the default mode "prefer" uses TLS without verifying when the server offers it and plain text otherwise, and
  does not fall back when the TLS handshake fails, so an operator sets `sslmode` on purpose; and that a host that adds
  a library built on `ring` must install a default crypto provider.

The simplest version: one line in `Cargo.toml`. Whether the driver's TLS is correct is the driver's business and is
not tested here.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `store-backends`: a store on PostgreSQL can be opened over TLS.

## Out of scope

- A PostgreSQL with TLS in Docker or CI, and tests of the driver's TLS modes.
- A feature that switches TLS off or on, or a choice of TLS library for the host.
- TLS for SQLite, which has no network.

## Order

The first one. After `private-store-trait` (released as 0.5.0), before `one-name-keys`. It is not a step of ADR 0018.

## Impact

- `crates/cicada/Cargo.toml`, `Cargo.lock`, one test in `crates/cicada`, `docs/operations.md`, `README.md` (the
  line on opening PostgreSQL), `CHANGELOG.md`, and the CI step that checks which driver each feature builds: it also
  checks that the SQLite build has no TLS library and that the workspace has one crypto provider.
- New dependencies of a PostgreSQL host: rustls, `aws-lc-rs` (its crate `aws-lc-sys` compiles the C library AWS-LC)
  and webpki-roots. rustls and `aws-lc-rs` are in the workspace already, through the HTTP client.
- A host whose other libraries build rustls on `ring` gets two crypto providers and must install a default at start.
- Not breaking: a URL that works today works the same, except that the default mode now uses TLS when the server
  offers it.
- No ADR: no guarantee and no wire format changes.
