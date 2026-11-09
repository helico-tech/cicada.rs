# 0016. Releases are tags on main

Date: 2026-11-09

## Status

Accepted

## Context

Other private Helico projects need a fixed version of this library. A plain git dependency follows `main`, which takes
breaking changes. The repository is private, and GitHub Packages has no Cargo registry.

## Decision

A release is a git tag `vX.Y.Z` on a commit of `main`, used as a git dependency with `tag = "vX.Y.Z"`.

- All crates in the workspace share one version and one tag. They change together, and mixing versions would give two
  copies of the same types.
- A GitHub Release is created by `.github/workflows/release.yml` and only after the tag, the workspace version and a
  dated changelog section agree, and CI and the end-to-end suite have passed on that commit.
- While the version is 0.x, a breaking change raises the minor number and anything else the patch number. A breaking
  change is one to the public Rust API, the HTTP wire format, the store schema, a contract hash or a guarantee in
  `docs/architecture.md`.
- A tag with a GitHub Release never moves and is never deleted. A correction is a new version. A tag whose checks failed
  was never released, and may be deleted.

There is no private Cargo registry. It would add real version ranges, but also a service to run or pay for, which a few
internal consumers do not justify.

## Consequences

- A consumer upgrades by changing one tag, and `Cargo.lock` pins the exact commit.
- Consumers need a read-only credential for this repository, on developer machines, in CI and in Docker builds. The
  README says how.
- Cargo's version ranges do not apply to tags: there are no automatic patch upgrades.
- The rule that a tag never moves is written down, not enforced by GitHub.
