# Release Cicada

Documentation-only updates from the dedicated docs session merge after checks without a library release,
version bump or tag (owner decision, 2026-11-14). Implementation changes use the procedure below.

A release is a tag `vX.Y.Z` on `main` (ADR 0016). All crates share the version in `[workspace.package]`.

1. On `main`, set the version in `Cargo.toml` and run `cargo check`, which updates `Cargo.lock`. A breaking change raises
   the minor number while the version is 0.x, anything else the patch number;
   [installation](install.md#upgrade) says what counts as breaking. `docs/openapi.json`
   carries the version: regenerate it (a test says how when it is stale) and update the README's `tag = "vX.Y.Z"`
   lines. The docs session refreshes the dependency examples in [installation](install.md); include the new version
   in the docs handoff rather than editing that guide from the builder worktree. Keep `metadata.cicada-version` in
   `skills/cicada/SKILL.md` equal to the workspace version; the skill test and release checker enforce it.
2. In `CHANGELOG.md`, replace `unreleased` in the heading of that version with today's date, as `YYYY-MM-DD`.
3. Commit and push to `main`, and wait for CI.
4. Run `scripts/release-check.sh vX.Y.Z`. It prints the release notes, or says what is wrong.
5. `git tag -a vX.Y.Z -m vX.Y.Z` and `git push origin vX.Y.Z`.

The push starts `.github/workflows/release.yml`: the same check, CI, then the end-to-end suite, and only then a GitHub
Release with the changelog section as its notes and `cicada-skill-vX.Y.Z.tar.gz` attached. The workflow packages
the skill from the tagged Git objects in the verify job and again before publication; missing files or mismatched
versions stop the release. After tagging, `python3 scripts/package-skill.py vX.Y.Z dist` reproduces the archive locally. If a check fails, no release exists yet: delete the tag
(`git push origin :refs/tags/vX.Y.Z` and `git tag -d vX.Y.Z`), fix the cause on `main` and tag again. A tag with a
GitHub Release is never moved or deleted; a correction is a new version.

When the next change lands, add a section `## X.Y.Z — unreleased` to the changelog and set that version in `Cargo.toml`
and in the skill metadata.

