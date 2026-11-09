# Design

## Context

- The repository `helico-tech/cicada.rs` is private. The workspace version is set once in `[workspace.package]`, and
  every crate has `publish = false`.
- `.github/workflows/ci.yml` runs format, clippy and `cargo test --workspace` with PostgreSQL. It leaves out the
  end-to-end suite (`CICADA_E2E=1`, about 3 minutes, needs Docker).
- `CHANGELOG.md` uses the heading `## 0.2.0 — unreleased`.
- The tag `v0.1.0` exists on an old commit, with no GitHub Release.
- No other Helico project depends on this repository yet.

## Goals / Non-Goals

**Goals:**

- A consumer gets exactly the code that passed all checks, and upgrades only by changing one line.
- Releasing is a few manual steps with one command to check them.

**Non-Goals:**

- Automating the version bump or the changelog. Releases are rare, and writing the changelog is the useful part.
- Supporting several release lines. Fixes go to the newest version only.

## Decisions

### A tag and a git dependency, not a registry

Consumers use `tag = "vX.Y.Z"`. Cargo records the exact commit in the consumer's `Cargo.lock`.

- *crates.io:* the code is private.
- *GitHub Packages:* it has no Cargo registry.
- *A private registry (Kellnr, Cloudsmith, Shipyard):* it adds real version ranges, but also a service to run or pay
  for, and publishing every crate in the right order. Not worth it for a few internal consumers.

### One version and one tag for the whole workspace

The crates depend on each other and change together. Separate versions would add bookkeeping and let a consumer mix
versions, which gives two copies of the same types and confusing compile errors.

### The release workflow gates on the same checks as CI, plus the end-to-end suite

`release.yml` runs on tags matching `v*.*.*`. Its jobs:

1. `verify`: runs `scripts/release-check.sh`.
2. `checks`: calls `ci.yml`, which gets a `workflow_call` trigger, so there is one definition of the checks.
3. `e2e`: runs `CICADA_E2E=1 cargo test -p cicada-e2e`. It is in the release and not in every push because it is slow
   and releases are rare.
4. `release`: needs all of the above, and creates the GitHub Release. Only this job has `contents: write`.

### A script for the agreement check

`scripts/release-check.sh vX.Y.Z` fails when the tag has the wrong form, the workspace version differs (read with
`cargo metadata --no-deps`), the changelog section is missing or says `unreleased`, or the commit is not on `main`. On
success it prints the changelog section, which becomes the release notes. A plain script can be run by hand before
tagging, which saves a failed run.

### Changelog headings

Between releases the top section keeps the form `## 0.3.0 — unreleased`, with the workspace version already set to
that number. Releasing replaces `unreleased` with the date.

### Access from other private projects

The dependency URL is always `https://github.com/helico-tech/cicada.rs`, because Cargo treats a different URL as a
different source. Access is set up around it:

- Cargo must use the git command line: `CARGO_NET_GIT_FETCH_WITH_CLI=true`, or `[net] git-fetch-with-cli = true`. Its
  built-in git does not use the usual credential helpers.
- **On a developer machine:** `gh auth setup-git`.
- **In GitHub Actions:** the default `GITHUB_TOKEN` only reaches its own repository. A fine-grained token with read
  access to this repository, kept as a secret, is used through a `git config --global url."…".insteadOf` rule for this
  repository's URL only, so the token is never sent for other dependencies. A deploy key is the alternative, but it
  would need an SSH URL.
- **In a Docker build:** the same token, passed as a BuildKit secret and set through `GIT_CONFIG_*` variables of that one
  step. `git config --global` would write it to a file in an image layer.

Consumers name the tag once, in `[workspace.dependencies]`.

## Risks / Trade-offs

- [The rule that a tag never moves is not enforced] → It is written in the docs. `Cargo.lock` still protects a consumer,
  because it pins the commit. A repository ruleset can be added later.
- [A release takes longer because of the end-to-end suite] → Releases are rare, and it is the only test of several
  nodes in Docker.
- [A failed release leaves a tag behind] → A tag without a GitHub Release was never released, so it may be deleted and
  pushed again.
- [A token in the consumer's CI has broad access] → It is a fine-grained token, read-only, for this one repository.
- [Downloading a whole repository for a few crates] → Cargo caches it, and the repository is small now that the console
  is gone.

## Migration Plan

1. Land this change. Nothing happens until a tag is pushed.
2. The tag `v0.1.0` stays as it is, without a GitHub Release.
3. When the open changes are done, set the date in the changelog and push `v0.2.0`. That first run is the real test of
   the workflow.
4. Rollback for a consumer is pinning an older tag.
