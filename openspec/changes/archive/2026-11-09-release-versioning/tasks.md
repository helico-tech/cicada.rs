# Tasks

## 1. Version check

- [x] 1.1 Write `scripts/release-check.sh vX.Y.Z`: tag form, workspace version, dated changelog section, commit on
      `main`; on success print the section. Verify: run it on the working tree for a wrong tag form, a wrong version and
      the current `unreleased` section; each fails with a clear message.
- [x] 1.2 Verify the success path: run the script against a temporary copy of `CHANGELOG.md` with a date in the heading;
      it prints exactly that section, without the next one.

## 2. Workflows

- [x] 2.1 Add a `workflow_call` trigger to `.github/workflows/ci.yml`. Verify: the file still has its `push` and
      `pull_request` triggers, and CI passes on the next push.
- [x] 2.2 Add `.github/workflows/release.yml` with the jobs `verify`, `checks`, `e2e` and `release`, with `contents: write`
      only on `release`. Verify: the YAML parses, and each job's `needs` is as in `design.md`. The first real tag is the
      full test.

## 3. Documentation

- [x] 3.1 README: replace the git dependency part of "Building and testing" with a section on using a release: pinning
      the tag once, and access on a developer machine, in GitHub Actions and in a Docker build. Verify:
      `cargo test -p cicada-examples --doc`.
- [x] 3.2 Check the documented setup: in a scratch project under the session's temporary folder, depend on `cicada` with
      `tag = "v0.1.0"` and run `cargo metadata` with `CARGO_NET_GIT_FETCH_WITH_CLI=true`. Verify: it resolves the private
      repository at that tag.
- [x] 3.3 `CHANGELOG.md`: state the heading format and the rule for version numbers at the top. Verify: the script
      still reads the `0.2.0` section.
- [x] 3.4 Write how to release, step by step, in `docs/operations.md`, and add one line to `AGENTS.md` pointing to it.
      Verify: `cargo test -p cicada-examples --doc`.
- [x] 3.5 Write an ADR with the next free number: releases are tags on `main`, with one version for the workspace, and
      the rule for version numbers. Link it from the README's decisions table. Verify: the link resolves.

## 4. Finish

- [x] 4.1 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
      `openspec validate release-versioning --strict`. Verify: all pass.

## Workflow follow-up

- Cutting `v0.2.0` is a separate step, after the open changes are done and when Arjan asks for it.
