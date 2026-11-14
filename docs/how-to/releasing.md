# Release Cicada

A release is a tag `vX.Y.Z` on `main` (ADR 0016). All crates share the version in `[workspace.package]`.

1. On `main`, set the version in `Cargo.toml` and run `cargo check`, which updates `Cargo.lock`. A breaking change raises
   the minor number while the version is 0.x, anything else the patch number;
   [installation](install.md#upgrade) says what counts as breaking. `docs/openapi.json`
   carries the version: regenerate it (a test says how when it is stale) and update the README's `tag = "vX.Y.Z"`
   lines and the dependency examples in [installation](install.md).
2. In `CHANGELOG.md`, replace `unreleased` in the heading of that version with today's date, as `YYYY-MM-DD`.
3. Commit and push to `main`, and wait for CI.
4. Run `scripts/release-check.sh vX.Y.Z`. It prints the release notes, or says what is wrong.
5. `git tag -a vX.Y.Z -m vX.Y.Z` and `git push origin vX.Y.Z`.

The push starts `.github/workflows/release.yml`: the same check, CI, then the end-to-end suite, and only then a GitHub
Release with the changelog section as its notes. If a check fails, no release exists yet: delete the tag
(`git push origin :refs/tags/vX.Y.Z` and `git tag -d vX.Y.Z`), fix the cause on `main` and tag again. A tag with a
GitHub Release is never moved or deleted; a correction is a new version.

When the next change lands, add a section `## X.Y.Z — unreleased` to the changelog and set that version in `Cargo.toml`.

