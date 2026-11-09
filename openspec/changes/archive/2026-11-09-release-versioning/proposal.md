# Proposal

## Why

Other private Helico projects should be able to depend on a fixed version of Cicada for Rust. Today the README tells
them to use a plain git dependency, which follows `main`, and `main` takes breaking changes. There is one old tag
(`v0.1.0`), no GitHub Release, no written rule for version numbers, and nothing that says how a project gets access to
this private repository.

## What Changes

- A release is a git tag `vX.Y.Z` on `main`. All crates in the workspace share that one version.
- Pushing a tag starts a release workflow. It checks that the tag, the workspace version and `CHANGELOG.md` agree, runs
  the full checks including the end-to-end suite, and only then creates a GitHub Release with the changelog section as
  its notes.
- A script does the agreement check, so it can also be run by hand before tagging.
- Written rules for version numbers: while the version is 0.x, a breaking change raises the minor number, anything else
  the patch number. A published tag never moves.
- The README explains how to use a release from another private project: pin the tag once in
  `[workspace.dependencies]`, and how to get access locally, in CI and in Docker builds.

The crates stay unpublished (`publish = false`). GitHub Packages has no Cargo registry, so a tag and a git dependency
are the simplest thing that works on GitHub alone.

## Capabilities

### New Capabilities

- `release-management`: how a version is released, what a version number promises, and how other projects depend on a
  release.

### Modified Capabilities

None.

## Out of scope

- Publishing to crates.io, or running a private Cargo registry.
- Tools that bump versions or write the changelog automatically.
- Building and publishing binaries or the sample-service Docker image.
- Cutting `v0.2.0` itself. That is a separate step, when the open changes are done and Arjan asks for it.
- Repository rulesets that protect tags. The rule that a tag never moves is written down, not enforced by GitHub.

## Impact

- New: `.github/workflows/release.yml`, `scripts/release-check.sh`.
- `.github/workflows/ci.yml` becomes callable from the release workflow.
- `README.md`, `CHANGELOG.md` (heading format), `AGENTS.md`, and one new ADR.
- No change to library code, the wire format or the store schema.
