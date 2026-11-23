# Tasks

Patch release 0.20.6, `Release: now` (design, decision 8). No public API, wire format, schema or guarantee changes.

- [x] 1. **The lockfile and the floors.** Run `cargo update` and commit `Cargo.lock`. Workspace `Cargo.toml`:
  `sqlx = "0.9.1"`. `crates/cicada/Cargo.toml`: an optional `libsqlite3-sys = { version = "0.39", default-features =
  false }` (workspace dependency if the workspace keeps its versions there), added to the `sqlite` feature
  (decision 2). If an update breaks a check, hold that crate back with `cargo update -p <crate> --precise <version>`
  at or above its fixed version, and name it in the report. Verify: `cargo tree -p cicada -e normal --features
  sqlite | grep libsqlite3-sys` shows 0.39 or later; `cargo check -p cicada --features sqlite`, `--features postgres`
  and alone; `cargo test -p cicada --features sqlite`; `bash scripts/ci/dependency-trees.sh` (its check that the
  `postgres` tree holds no `libsqlite3-sys` already covers the new dependency).
- [x] 2. **The advisory check.** A new `deny.toml` at the root with only `[advisories]` and an ignore of
  RUSTSEC-2026-0543 (`chrono`, unmaintained) with its reason (decision 3). `scripts/gate-policy.json`: a check
  `advisories` in `full`, `["cargo", "deny", "check", "advisories"]` (decision 4). `.github/workflows/ci.yml`: a step
  `cargo install cargo-deny --locked --version 0.20.2` before the full checks (decision 5). Update any policy test
  in `scripts/test_gate_policy.py` that lists the full checks. Verify: `cargo deny check advisories` passes;
  `python3 -m unittest discover -s scripts -p 'test_gate*.py'`.
- [x] 3. **Version, changelog and docs handoff.** Set 0.20.6 in every version place, as a patch release does
  (`builder.md`), with a `## 0.20.6 — unreleased` changelog section: the advisories fixed, the new floors, and that
  full gate checks now need `cargo-deny` 0.20.2. Write `docs-handoff.md` from the design's "Docs it breaks"; nothing
  blocks. Verify: `cargo test -p cicada-examples --test skill` (the skills' versions), `--test docs`.
- [ ] 4. **Finishing checks.** `openspec validate dependency-advisories --strict`, commit, then `python3 scripts/gate.py
  run` (the full policy, now with `advisories`; no E2E: the release right after runs it).
