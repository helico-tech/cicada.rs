# Docs handoff for local-push-gate

Version: 0.18.9. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

No library change: API, wire formats, contract hashes, store schema and guarantees are those of 0.18.8. Contributor
tooling changed (ADR 0046):

- `scripts/gate.py` with `checks`, `run`, `status`, `recover`, `install-hook`, `pre-push` and `release`; its policy
  `scripts/gate-policy.json`; the multi-line checks `scripts/ci/feature-builds.sh` and `scripts/ci/dependency-trees.sh`.
  Tests: `scripts/test_gate_policy.py`, `test_gate_evidence.py`, `test_gate_hook.py`, `test_gate_release.py`, with the
  helper `scripts/gate_testing.py`. All Python tests in `scripts/` (`test_*.py`) now run in the full policy.
- `.github/workflows/ci.yml` is manual only (`workflow_dispatch`); `.github/workflows/release.yml` is deleted.
- `scripts/release-check.sh [--before-push] vX.Y.Z` checks the checked-out commit; a local tag must point at it.
- ADR 0016 is marked superseded in its release-workflow and failed-tag-deletion clauses; ADR 0046 is accepted.
- The build loop's Land step and the builder brief use the gate (`.claude/skills/build-loop/`, review brief).

## 1. AGENTS.md

**Where.** `AGENTS.md`, `Build and test`, `Map` (row `scripts/`) and `Rules` (the Releases bullet).
**Text.** Add to Build and test:

```sh
python3 scripts/gate.py run --e2e      # check the committed HEAD in .claude/worktrees/gate and record evidence (PostgreSQL, Docker)
python3 scripts/gate.py status         # what evidence HEAD has, and what a push of it needs
python3 -m unittest discover -s scripts -p 'test_*.py'   # the Python tests of the scripts, gate included
```

The `scripts/` row gains: `gate.py`, the local gate (evidence, pre-push hook, release; ADR 0046), its policy
`gate-policy.json` and `ci/`, the checks the policy runs. The Releases rule becomes: releases are tags on `main`,
published with `python3 scripts/gate.py release vX.Y.Z` by the build loop on the owner's yes; a remote version tag is
never moved or deleted, released or not. Drop any mention of CI running on push.
**Caveats.** Pushing `main` needs gate evidence for the pushed tip once the hook is installed, also for the docs
session.

## 2. Releasing

**Where.** `docs/how-to/releasing.md`, whole page (steps 3 to 5 and the paragraph about `.github/workflows/release.yml`).
**Text.** Steps 1 and 2 stay. Then: commit the release on `main`, and run `python3 scripts/gate.py release vX.Y.Z` in the
main checkout. It pins the commit (HEAD, or an explicit REV), runs `scripts/release-check.sh --before-push` there,
saves the notes, reuses or runs the checks and the E2E suite, pushes that commit to `origin` `main` without force
(skipping it when origin's main already contains it, stopping when it diverged, refreshing once after a rejection),
runs `release-check.sh` in full, creates the annotated tag at the commit (or accepts one already there), packages the
skill and checks its `SOURCE.json` and files against the commit, pushes that exact tag object, then creates a draft
GitHub Release in origin's repository, checks its tag, title, notes and single asset hash, and publishes it.
Recovery: a failure before the tag push leaves no remote tag. After it, run the same command again: state is kept in
`<git common dir>/cicada-gate/release/vX.Y.Z.json`, the retry refuses another commit, verifies the same raw tag object
and resumes a matching draft. A conflicting draft, another tag object or a diverged main stops it without
overwriting. A published Release is reported as already published and never changed. A version tag on origin is never
deleted or moved, also when its checks failed before a Release existed: release a new version instead. The pinned
commit of a release that pushed nothing can only change by removing that state file by hand; say so only as owner
recovery.
**Caveats.** Needs local PostgreSQL (`CICADA_GATE_PG_URL`, default the compose URL on port 55432), Docker for E2E,
`jq`, `gh` signed in with release rights, and Python 3.11+. Main and tag pushes are not rolled back when a later
GitHub step fails; publication is not atomic across Git and GitHub.

## 3. The local gate

**Where.** `docs/how-to/releasing.md` (or a new section `Local checks and the pre-push gate` in
`docs/maintaining/maintenance.md`; the docs session decides) and `docs/operations.md`, `Releasing`.
**Text.**
- *Coverage.* `run` uses the full policy: Python tests of `scripts/`, `cargo fmt --all --check`, clippy with
  `-D warnings`, `cargo test --workspace` with PostgreSQL, cicada with no feature/sqlite/postgres/testing and
  cicada-app with no feature/sqlite/postgres/tui built alone with warnings denied, `cargo test -p cicada-app`, and the
  driver, TLS, crypto-provider and terminal-library dependency-tree checks. `--e2e` adds the unfiltered Docker suite.
- *Evidence.* Records in `<git common dir>/cicada-gate/records/<kind>-<tree>.json` name the commit and tree, the
  policy and runner blobs, the exact commands and exit codes, timestamps, tool versions and PostgreSQL host, port and
  database (never credentials). A record proves that this tree passed these commands in the recorded environment; it
  does not expire and claims nothing about other toolchains. `--force` runs again; a forced run that fails or is
  interrupted never falls back to the old pass.
- *Docs shortcut.* A commit whose only changes since a direct full pass (at most 100 first-parent commits back) are
  regular, non-executable Markdown under `README.md`, `CHANGELOG.md`, `AGENTS.md`, `docs/**/*.md` except
  `docs/reference/`, `skills/cicada/**/*.md`, `.claude/skills/**/*.md` or `openspec/**/*.md` gets fresh docs checks
  (Python tests, doctests, docs and skill tests) instead of full ones, and may reuse that base's E2E pass. Anything
  else (a crate README, OpenSpec YAML, `docs/reference/`, a mode change, a symlink, a rename out of these paths, the
  review loop's `.taken` files) needs full checks. So does a policy or gate change.
- *The hook.* `python3 scripts/gate.py install-hook` installs `pre-push` (respecting `core.hooksPath`, never replacing a
  foreign hook; chain it instead). It refuses: a push of `main` whose tip has no checks evidence, a non-fast-forward or
  deleted `main`, an unknown remote main (fetch first), a new `vX.Y.Z` tag without checks and E2E or not on the named
  remote's previously fetched `main` (push main first, through a named remote), and any change or deletion of an
  existing remote version tag. Other refs pass.
- *Owned worktree and environment.* Checks run in `.claude/worktrees/gate` (created by the gate, must be git-ignored,
  never adopted if it exists unowned), detached at the commit and cleaned except its `target/` cache. Only the
  environment's PATH, HOME, user, shell, TMPDIR, locale, TERM, Cargo/rustup homes and toolchain, Docker connection and
  SSH agent pass through; update flags, test filters, `RUSTFLAGS`, wrappers and `CARGO_TARGET_DIR` are dropped. A
  Cargo config above the worktree or in `CARGO_HOME`, or a toolchain file above it, is refused. A check that changes
  the checkout fails.
- *Interruption and recovery.* One run at a time: a busy lock refuses. Ctrl-C or TERM stops the command's process
  group and records no pass, and leaves the in-progress marker. `python3 scripts/gate.py recover` clears it only after
  checking that the gate process and its recorded command group are gone (it stops an identified group) and that no
  process works in the owned worktree; otherwise it refuses. Interrupted or failed records stay unusable until a new
  run passes. A forced run makes the old record unusable before anything that can fail, so a refusal or an
  interruption never leaves the old pass standing. The release runs its remote git calls, `release-check.sh`, the
  skill packaging and `gh` the same way: a killed release leaves the lock held while such a command still runs, so
  `recover` and a second release refuse until it ends. Docker containers are outside this: the E2E harness removes
  its own stack.
- *Manual Linux CI.* Actions → CI → Run workflow runs `python3 scripts/gate.py checks --scope full` with PostgreSQL. It
  records no evidence and publishes nothing; a local macOS pass is not Linux coverage.
- *Limits.* This is local workflow control: `--no-verify`, another clone, an old hook or a forged record bypasses it;
  there is no server enforcement, no hermetic build and no signature.
**Caveats.** Records and the gate worktree are shared by every session of the clone; one gate runs at a time.

## 4. Docs maintenance

**Where.** `docs/maintaining/maintenance.md`, where docs merges reach `main`.
**Text.** Before pushing a docs-only merge to `main`, run `python3 scripts/gate.py run` on it: it runs the docs checks
over the last full pass. Documentation-only maintenance still needs no version bump or tag.
**Caveats.** A docs merge that also touches a non-Markdown file needs full checks.

## Blocking

None. No doctest, link, guarantee row or skill route depends on new prose: `cargo test -p cicada-examples --doc`,
`--test docs` and `--test skill` pass on this branch.

## Status

Proposed text only; none of this prose is written.

Checks run by the builder: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` in
the worktree; `cargo test -p cicada-examples --doc --test docs --test skill` (47 passed); `openspec validate
local-push-gate --strict` (valid); the 64 Python tests of `scripts/`. The gate itself ran in a temporary clone of this
branch, never in the shared repository (so no evidence of it exists in the real common directory): `gate.py run
--e2e` passed full and E2E on `58290e4` (60 Python tests, 3055 Rust tests including PostgreSQL, every feature-alone
build and dependency-tree check, 28 E2E tests in Docker). Review round 1 changed `scripts/gate.py` and its tests
afterwards, so that evidence does not cover the current head; nothing outside `scripts/` and Markdown changed since.
A bad `CICADA_PG_URL` fails the PostgreSQL tests (checked by hand with `cargo test -p cicada --test store`).

Installation after landing: the build loop runs `python3 scripts/gate.py install-hook` once in the main checkout, with
the owner's yes, then releases 0.18.9 with `python3 scripts/gate.py release v0.18.9`. The gate needs
`.claude/worktrees/` ignored (it is, in `.git/info/exclude`) and no Cargo config or toolchain file above the checkout
or in `~/.cargo` (none today).
