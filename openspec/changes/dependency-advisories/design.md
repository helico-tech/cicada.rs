# Design

Decided with the owner, 2026-11-23. Evidence: `cargo deny check advisories` (cargo-deny 0.20.2) on `main` at `3f639cf`
reports 11 advisories: the 10 that `cargo update` fixes and `chrono`. `cargo update --dry-run` makes 100 updates,
among them `sqlx` 0.9.0 → 0.9.1 (`libsqlite3-sys` 0.37.0 → 0.40.0), `rustls` 0.23.46, `rustls-webpki` 0.103.17,
`tokio` 1.54.1, `ctutils` 0.4.8 and `url` 2.5.9.

## Decisions

1. **Update the whole lockfile with `cargo update`.** Consumers resolve the newest compatible versions, so the repo's
   tests should build with those too, and one update also catches fixes that have no advisory yet. The full gate
   checks the result. If one update breaks a check, the builder holds that crate back with
   `cargo update -p <crate> --precise <version>`, at or above the version that fixes its advisory, and names it in
   its report. Not `cargo update -p` for each advised
   crate only: a smaller diff, but the repo keeps testing older versions than consumers get.
2. **Force the fixed SQLite for consumers with the `sqlite` feature.** `libsqlite3-sys` is a `links` crate (one
   version per build). `sqlx` 0.9.0 holds it below 0.38; `sqlx-sqlite` 0.9.1 allows `>=0.30.1, <0.41.0`, so a floor
   on `sqlx` alone lets a consumer keep 0.37.0 from an old lockfile, or get it when another crate holds it back
   (review `dependency-advisories.design.1`). So: the `sqlx` floor goes to `"0.9.1"`, and `crates/cicada` gets an
   optional direct dependency `libsqlite3-sys = { version = "0.39", default-features = false }`, turned on by the
   `sqlite` feature. A consumer whose other crates need an older `libsqlite3-sys` now gets a resolution error instead
   of the CVE. The `dependency-trees` check keeps it out of the `postgres` tree. Not the `sqlx` floor alone with a
   note to consumers: the proposal's point is that the lockfile does not protect them. Not raising `tokio` and the
   others: a plain `cargo update` in the consumer's project reaches their fixes, and nothing in Cicada needs them.
3. **Accept `chrono` for now.** It is in the public API (timestamps in the store and the wire model) and a dependency
   of `sqlx`; no maintained successor is settled. `deny.toml` ignores RUSTSEC-2026-0543 with that reason. Not a
   replacement now: a breaking change of its own, out of scope.
4. **`cargo deny check advisories` becomes a check of the gate's `full` kind**, named `advisories`, with a `deny.toml`
   at the root that sets only the `[advisories]` section and the one ignore. It catches the next advisory before a
   push. A batched release may reuse the full pass from landing (ADR 0046), so an advisory published between
   landing and release is caught by the next full run, not by the release. Not a check at release only: an advisory found at release blocks the release after the batch has landed.
   Not a separate script: the policy names the command directly, as it does for `fmt` and `clippy`.
5. **The check needs `cargo-deny` and network access.** It fetches the advisory database each run; without network
   or without `cargo-deny` installed, the check fails and the gate refuses evidence (fail closed). The docs session
   adds `cargo-deny` to the gate's prerequisites; the manual Linux workflow (`.github/workflows/ci.yml`) installs it
   with `cargo install cargo-deny --locked --version 0.20.2` before its full checks; the prerequisites name the same
   version, since its configuration changes between 0.x versions. The gate passes no proxy variables, so the check
   cannot run behind an HTTPS proxy; that is accepted for now. Not `--disable-fetch`: a stale database passes a commit it should
   stop.
6. **A new advisory is handled as a change of its own**: an update of the lockfile (with `Release: now` for a
   vulnerability) or an ignore in `deny.toml` with its reason, as here for `chrono`. Until then, full gate runs fail;
   a documentation-only push still reuses an earlier full pass (the docs kind does not run the check).
7. **The `release-management` spec names the check.** Its requirement "A release passes all checks first" lists the
   full checks, so its delta adds dependency advisories to that list.
8. **Normal lane, `Release: now`, patch 0.20.6.** The `Cargo.toml` floor and the gate policy are not on the fast-lane
   list. No public API, wire format, schema or guarantee changes, so a patch. The builder runs `gate.py run` without
   `--e2e`; the release right after runs E2E against the new lockfile.

## What it touches

- `Cargo.lock`; the `sqlx` line of the workspace `Cargo.toml`; the optional `libsqlite3-sys` of `crates/cicada`
  and its `sqlite` feature. `scripts/ci/dependency-trees.sh` already refuses `libsqlite3-sys` in the `postgres` tree.
- `deny.toml`, new; `scripts/gate-policy.json` (a check `advisories` in `full`); any test of the policy's check list
  in `scripts/test_gate*.py`; a step that installs `cargo-deny` in `.github/workflows/ci.yml`.
- `CHANGELOG.md` (0.20.6 unreleased) and the version places for a patch.

## Docs it breaks

No doctest, link, guarantee row or skill route. For the docs session:

- `docs/how-to/releasing.md`, the prerequisites: `cargo-deny` installed (`cargo install cargo-deny --locked`) and
  network access for the advisory database; version 0.20.2; the full checks include `advisories`.
- `AGENTS.md`, "Build and test": `cargo deny check advisories` and that full gate checks need `cargo-deny`.
- `docs/how-to/install.md` or the changelog note: with the `sqlite` feature, Cicada now needs `sqlx` 0.9.1 and
  `libsqlite3-sys` 0.39 or later; a project that holds `libsqlite3-sys` lower gets a resolution error.
