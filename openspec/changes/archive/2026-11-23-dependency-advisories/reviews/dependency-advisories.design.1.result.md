Verdict: not yet

## Blocking

1. **The `sqlx` floor does not keep consumers off the bad SQLite.** Where: proposal.md, What Changes ("cannot resolve the SQLite with the CVE"); design.md, decision 2. In the crates.io index, `sqlx-sqlite` 0.9.1 asks for `libsqlite3-sys >=0.30.1, <0.41.0`. So 0.37.0 still fits. A consumer who updates Cicada with an existing lockfile gets `sqlx` 0.9.1 and keeps `libsqlite3-sys` 0.37.0. A consumer whose other crates pin `libsqlite3-sys` ≤0.38 also gets the bad version, with no error, because it is a `links` crate (only one version can be in a build). Decision 2 is right that 0.9.1 *allows* the fix. The proposal says it *forces* the fix. Choose one:
   - (a) Add an optional direct dependency `libsqlite3-sys = ">=0.39"` to `crates/cicada`, turned on by the `sqlite` feature. This forces the fix. It costs one more dependency line. The `dependency-trees` check still passes, because the postgres tree stays free of it.
   - (b) Keep only the floor. Fix the wording, and have the docs handoff tell consumers to run `cargo update -p libsqlite3-sys`.
   I prefer (a), because the Why says the lockfile does not protect consumers.

## Notes

- Decision 1: say that a held-back crate must still be at or above its fixed version.
- Decision 5: the gate drops `HTTPS_PROXY` (it is not in `KEEP_ENV`), so behind a proxy the check always fails. Pin a `cargo-deny` version in `ci.yml` and in the prerequisites, because the config format changes between 0.x versions.
- Decision 4: a release reuses the full pass from landing (ADR 0046). An advisory published between landing and a batched release is not caught. That is acceptable, but the design should say so.
- Proposal: "passes, apart from `chrono`" should now read "passes; `deny.toml` accepts `chrono`". Also, `rustls` comes in through both `reqwest` and `sqlx`.

## Questions for the owner

- F1: choose (a) or (b) above. My pick: (a).

## What I checked and did not check

Checked:
- `cargo deny check advisories` at `3f639cf` shows the same 11 advisories, with or without `--all-features`.
- The `cargo update --dry-run` versions match the design.
- When the fetch fails, the check exits with code 1 (it fails closed).
- The spec delta is an exact copy of the spec, plus the new text.

Not checked:
- I did not run the gate on an updated lockfile.
- I did not check the E2E images.
- I did not check the `deny.toml` syntax.
