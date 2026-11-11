# Design

Described with the owner on 2026-11-11 (change loop, Describe). The owner chose a lighter loop for this change: no
design review, but a builder and a validator as usual (decision 5). Changed by the owner on 2026-11-11 after the first
build and its validation: rustls on `aws-lc-rs` instead of `ring` (decisions 1 to 3).

## Context

- The workspace names the driver as `sqlx = { version = "0.9.0", default-features = false, features =
  ["runtime-tokio", "json", "chrono"] }`. sqlx 0.9.0 compiles TLS in only with one of its TLS features:
  `tls-rustls-ring-webpki` (also named `tls-rustls`), `tls-rustls-ring-native-roots`, `tls-rustls-aws-lc-rs` or
  `tls-native-tls`. Without one, a URL that asks for TLS fails with "SQLx was built without TLS support enabled".
- The two `ring` features build rustls on the crypto provider `ring`. `tls-rustls-aws-lc-rs` builds it on `aws-lc-rs`
  and trusts the built-in list of public CAs (webpki-roots); sqlx 0.9.0 has no feature for `aws-lc-rs` with the
  certificate store of the operating system.
- The `postgres` feature of `cicada` is `["dep:sqlx", "sqlx/postgres"]`; the `sqlite` feature is
  `["dep:sqlx", "sqlx/sqlite"]`.
- The driver reads these parameters from a PostgreSQL URL: `sslmode` (`disable`, `allow`, `prefer`, `require`,
  `verify-ca`, `verify-full`), `sslrootcert`, `sslcert` and `sslkey`; for each one the URL does not name, it reads
  `PGSSLMODE`, `PGSSLROOTCERT`, `PGSSLCERT` or `PGSSLKEY` from the environment. Its default mode is `prefer`: try TLS
  first, and go on without it when the server offers none.
- rustls is in the workspace already, on `aws-lc-rs`: the HTTP client of `cicada-client` and `cicada-cli` (reqwest's
  feature `rustls`).
- The tests and CI run one PostgreSQL without TLS, on port 55432 (`CICADA_PG_URL`).

## Goals / Non-Goals

Goals:

- A host that names the `postgres` feature can open a store over TLS, with nothing else to switch on.

Non-Goals:

- Testing the driver's TLS: which modes verify what is the driver's business.
- A PostgreSQL with TLS in Docker or in CI.

## Decisions

### 1. rustls on `aws-lc-rs`

The driver is built with rustls on `aws-lc-rs`, the crypto provider of the HTTP client, so the crates of Cicada agree
on one provider. Two providers in one build are a trap: rustls with both `ring` and `aws-lc-rs` cannot pick a
process-wide provider from its features, so every caller that leaves the choice to rustls panics unless the host
installed a default first. `rustls::ClientConfig::builder()` is such a caller, and so is the driver's `verify-ca` path
(`WebPkiServerVerifier::builder`). CI fails when the workspace builds rustls with two providers.

Not chosen: rustls on `ring` (chosen first; it puts a second provider next to the HTTP client's), and the TLS library
of the operating system (build images would need its headers).

### 2. The built-in list of public CAs, and `sslrootcert` for a private CA

When a URL asks to verify the server, the driver trusts the built-in list of public CAs and the CA file that
`sslrootcert` names. A database rarely has a certificate of a public CA, so a database with a private or
cloud-provider CA needs `sslrootcert`. The driver feature is `tls-rustls-aws-lc-rs`.

Not chosen: the certificate store of the operating system. It was chosen first, so that installing the CA on the host
would be enough, and dropped: sqlx 0.9.0 offers it only on `ring` (`tls-rustls-ring-native-roots`), which decision 1
rules out.

### 3. TLS comes with the `postgres` feature

`postgres` becomes `["dep:sqlx", "sqlx/postgres", "sqlx/tls-rustls-aws-lc-rs"]`. The `sqlite` feature stays as it
is, so a SQLite host builds no TLS for its store; CI fails when it does.

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

Decided by the main session, and widened by the owner with the change of decisions 1 to 3.

`docs/operations.md` gets a paragraph on TLS for PostgreSQL: the modes of `sslmode` in one line each; that the trusted
CAs are the built-in list of public CAs and the file that `sslrootcert` names, so a private or cloud-provider CA needs
`sslrootcert`; that the driver reads `PGSSLMODE`, `PGSSLROOTCERT`, `PGSSLCERT` and `PGSSLKEY` when the URL names
none; that the default `prefer` uses TLS without verifying when the server offers it and plain text otherwise, and does
not go back to plain text when the server offered TLS and the handshake fails (`sslmode=disable` is the way back); so
an operator who wants TLS sets `sslmode=verify-full`, or at least `require`. One sentence says that the crates of
Cicada build rustls on `aws-lc-rs`, and that a host that adds a library built on `ring` has two providers and must
install a default at start. The README names the paragraph where it shows how to open PostgreSQL.

## Risks / Trade-offs

- **A URL without `sslmode` now uses TLS when the server offers it**, without verifying the server. → It was plain
  text before, so nothing gets weaker. The docs say to set the mode on purpose. `PGSSLMODE` can set it too.
- **A private CA is not trusted from the host's certificate store.** → `sslrootcert` names it (decision 2).
- **A host whose other libraries build rustls on `ring` gets two providers**, and every caller that leaves the choice
  to rustls panics. → The host installs a default provider at start
  (`rustls::crypto::aws_lc_rs::default_provider().install_default()`); the docs say so.
- **A PostgreSQL host builds more crates**, among them `aws-lc-sys`, which compiles the C library AWS-LC. → rustls
  and `aws-lc-rs` are in the workspace already through the HTTP client.
- **The test proves that TLS is compiled in, not that a TLS connection works.** → Accepted (decision 4).

## Migration Plan

None. A host changes nothing; a URL that asks for TLS starts to work.
