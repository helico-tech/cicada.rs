# Proposal

## Why

Every push to `main` runs GitHub's CI (about 11 minutes), docs-only and proposal-only pushes included. Every tag runs
the release workflow, which repeats that CI and adds the Docker end-to-end suite (about 25 machine-minutes). The repo is
private, so these are paid minutes: on 2026-11-14 alone there were 15 CI runs and 7 releases, about 330 minutes. When
billing stopped the runs, landing stopped too: v0.17.4 has a tag but no GitHub Release, and 0.18.0 cannot be tagged.

Nothing is published to a registry (ADR 0016): consumers pin the git tag. The GitHub Release only carries the notes
and the skill archive (ADR 0041). Builders already run the full suite with PostgreSQL before review. The remote runs
mostly repeat work that was already done, so the owner asks to replace them with a local gate.

## What Changes

- **Push gate.** A local `pre-push` hook allows a push to `main` only with evidence that the checks passed for what is
  pushed. A script runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` with PostgreSQL. It records the result: the commit and tree, the commands, pass or fail,
  the toolchain and the time. The record is kept in Git's common directory, so every worktree and session shares it.
  The hook refuses a push to `main` with no passing record.
- **What evidence covers.** The design decides how a commit that changes only docs, `openspec/` or release text
  (README pins, changelog date) is treated. Options: no checks, the docs tests only, or a record keyed to the code
  paths, so that a release commit on a tested tree needs no new run.
- **Releases stay tags** (ADR 0016). `scripts/release-check.sh` and the end-to-end suite in Docker run locally. The
  GitHub Release, with its notes and the skill archive, is created locally with `gh release create`; that is an API
  call, not an Actions run.
- **GitHub workflows:** no CI on push or pull request, and no release workflow on a tag. Keep a manual
  (`workflow_dispatch`) run if the design finds one useful.
- A new ADR supersedes ADR 0016's rule that the release workflow creates the GitHub Release after CI and the end-to-end
  suite pass remotely. The build loop's landing steps, `scripts/release-check.sh`, AGENTS.md and (through the docs
  session) `docs/how-to/releasing.md` follow it.

## Out of scope

Server-side enforcement: a local hook is a workflow control. It can be bypassed (`--no-verify`, another clone) and is
not security. Changing which checks run. Publishing to a registry.

## Order

After `pg-close-test-flake`. Before `webhook-sink`.

## Impact

`.github/workflows/`, a new script and hook under `scripts/`, `scripts/release-check.sh`, `.claude/skills/build-loop/`,
AGENTS.md, a new ADR, and handoffs for the release how-to. No library code, API, wire format, schema or guarantee
changes.
