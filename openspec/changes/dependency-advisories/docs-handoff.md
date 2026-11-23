# Docs handoff for dependency-advisories

Version: 0.20.6. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

No library behaviour, public item, wire format, schema or guarantee changes. What users and maintainers see:

- `Cargo.lock` is updated (`cargo update`); every RustSec advisory it had is fixed except `chrono`
  (RUSTSEC-2026-0543, unmaintained, no fixed version), which `deny.toml` accepts with its reason.
- With the `sqlite` feature, `cicada` needs `sqlx` 0.9.1 (the workspace floor) and `libsqlite3-sys` 0.39, a new
  optional dependency of `crates/cicada` turned on by `sqlite` and not used in code. A consumer whose other crates
  hold `libsqlite3-sys` below 0.39 gets a resolution error instead of the SQLite with CVE-2026-119375. The
  `postgres` tree stays without it (`scripts/ci/dependency-trees.sh`).
- The gate's `full` kind has a new check `advisories`: `cargo deny check advisories`, configured by `deny.toml` at the
  root (only `[advisories]`). It needs `cargo-deny` 0.20.2 and network access; without either the check fails and the
  gate gives no evidence. `.github/workflows/ci.yml` installs it with `cargo install cargo-deny --locked --version
  0.20.2`. Proven by `scripts/test_gate_policy.py`
  (`test_advisories_are_checked_and_each_accepted_one_has_a_reason`, `test_only_a_manual_ci_workflow_remains`).
- The lockfile holds `schemars` at 1.2.2 (`cargo update -p schemars --precise 1.2.2`): 1.2.3 orders the keys of a
  contract's text differently, which fails `contracts_are_the_texts_recorded` (`sample-service`), the byte-for-byte
  text that `GET /types` serves. The contract hashes do not change. A consumer that runs `cargo update` gets 1.2.3 and
  so different texts from `GET /types` than this repository records.

## 1. The gate's prerequisites

**Where.** `docs/how-to/releasing.md`, `Local checks and the pre-push gate` (the prerequisites).
**Text.** Full checks need `cargo-deny` 0.20.2, installed with `cargo install cargo-deny --locked --version 0.20.2`
(the configuration changes between 0.x versions, so use this version), and network access: the check `advisories`
fetches the RustSec advisory database each run. The full checks include `advisories`. A new advisory makes every full
run fail until it is handled as a change of its own: a lockfile update (`Release: now` for a vulnerability) or an
ignore in `deny.toml` with its reason. A documentation-only push still reuses an earlier full pass.
**Caveats.** The gate passes no proxy variables, so the check cannot run behind an HTTPS proxy. A batched release may
reuse the full pass from landing, so an advisory published after landing is caught by the next full run, not by the
release.

## 2. AGENTS.md, Build and test

**Where.** `AGENTS.md`, `Build and test`.
**Text.** Add `cargo deny check advisories   # RustSec advisories in Cargo.lock; deny.toml lists accepted ones` to the
command block, and to the paragraph after it: full gate checks also need `cargo-deny` 0.20.2 and network access.

## 3. The SQLite floor for consumers

**Where.** `docs/how-to/install.md` (or wherever the docs session keeps dependency requirements).
**Text.** With the `sqlite` feature, Cicada 0.20.6 needs `sqlx` 0.9.1 and `libsqlite3-sys` 0.39 or later; a project
that holds `libsqlite3-sys` lower gets a resolution error. Run `cargo update` in your project as well: Cicada's
lockfile does not choose your versions.
**Caveats.** The requirement is written `"0.39"`, so it also keeps `libsqlite3-sys` below 0.40 in a consumer's build.
Do not tell consumers to expect byte-identical `GET /types` texts after their own `cargo update`: `schemars` 1.2.3
reorders keys (the hashes stay the same).

## Blocking

None.

## Status

Proposed text only; no prose is written yet. Checks run: see the change's tasks and the builder's report.
