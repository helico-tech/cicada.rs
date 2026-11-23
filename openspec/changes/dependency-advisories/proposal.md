# Proposal

## Why

The repository is public since 2026-10. `cargo deny check advisories` (2026-11-18) reports these in `Cargo.lock`:

- `rustls` 0.23.45: RUSTSEC-2026-0346 (`Debug` shows TLS secrets), 0347, 0348 and 0349. Fixed in 0.23.46.
- `rustls-webpki` 0.103.15: RUSTSEC-2026-0382 (distrusted intermediates accepted). Fixed in 0.103.16.
- `webpki-root-certs` 1.0.9: RUSTSEC-2026-0383 (distrusted emSign roots). Fixed in 1.0.10.
- `libsqlite3-sys` 0.37.0, through `sqlx` 0.9.0: RUSTSEC-2026-0459 (C SQLite CVE-2026-119375). Fixed in 0.39.0.
- `tokio` 1.53.2: RUSTSEC-2026-0386 (unsound `AsyncFd::new`). Fixed in 1.53.3.
- `ctutils` 0.4.3: RUSTSEC-2026-0385 (not constant time). Fixed in 0.4.4.
- `url` 2.5.8, through `reqwest`: RUSTSEC-2026-0592. Fixed in 2.5.9 (found 2026-11-22).
- `chrono` 0.4.45: RUSTSEC-2026-0543, unmaintained. No fixed version.

A plain `cargo update --dry-run` reaches a fixed version of each, except `chrono`: `sqlx` 0.9.1 brings
`libsqlite3-sys` 0.40.0. The lockfile is what the repo's own tests, the sample service, the E2E images and the
`cicada` binary build with. Consumers resolve their own versions, so the lockfile alone does not protect them.

## What Changes

- Run `cargo update` and commit the new `Cargo.lock`, after the full gate passes.
- Raise the floor in `Cargo.toml` from `sqlx = "0.9.0"` to `"0.9.1"`, and add a floor on `libsqlite3-sys` 0.39 with
  the `sqlite` feature, so consumers with that feature cannot resolve the SQLite with the CVE.
- `cargo deny check advisories` passes; `deny.toml` accepts `chrono`.

To decide in the design:

- Whether to raise other floors too (`tokio`, `rustls` through `sqlx` and `reqwest`), or only `sqlx`, the one with a C library CVE.
- What to do with `chrono`: accept it for now, or plan its replacement. It is part of the public API (timestamps in
  the store and the wire model) and a direct dependency of `sqlx`, so a replacement is its own change.
- Whether `cargo deny check advisories` becomes a gate check, with a `deny.toml` that lists the accepted `chrono`
  advisory. It catches the next advisory at push time, but a new advisory then blocks every push until it is handled.

## Out of scope

Replacing `chrono`. A `cargo deny` license policy: every dependency today has a permissive license or a permissive
choice. Automated dependency pull requests: Actions are off and changes go through the loops. Any change to
behaviour, public API, wire format, schema or guarantees.

## Lane

Release: now. It fixes known vulnerabilities in a public repository.

## Order

After `lighter-loops`, before `container-runtime`. It comes right after the process change because it fixes known
advisories in a public repository.

## Impact

`Cargo.lock`, the `sqlx` line in `Cargo.toml` and an optional `libsqlite3-sys` in `crates/cicada`; a gate check and `deny.toml` if the design picks one. A patch
release. No ADR. The docs handoff may name the new minimum `sqlx` for consumers; no doc block or guarantee row breaks.
