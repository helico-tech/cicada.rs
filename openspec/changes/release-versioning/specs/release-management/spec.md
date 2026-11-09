## Purpose

How a version of Cicada for Rust is released, what its number promises, and how another project depends on it.

## ADDED Requirements

### Requirement: A release is a version tag on main

A release SHALL be a git tag `vX.Y.Z` on a commit of `main`. Every crate in the workspace SHALL have that version.

#### Scenario: The tag matches the workspace version

- **WHEN** the tag `v0.3.0` is pushed and the workspace version is `0.3.0`
- **THEN** the version check passes

#### Scenario: The tag does not match the workspace version

- **WHEN** the tag `v0.3.0` is pushed and the workspace version is `0.2.0`
- **THEN** the release fails and no GitHub Release is created

#### Scenario: The tagged commit is not on main

- **WHEN** a version tag is pushed on a commit that is not on `main`
- **THEN** the release fails and no GitHub Release is created

### Requirement: A release has a dated changelog entry

`CHANGELOG.md` SHALL have a section for the released version, with its release date. The GitHub Release notes SHALL be
that section.

#### Scenario: The section has a date

- **WHEN** the changelog has the heading `## 0.3.0 — 2026-12-01` and `v0.3.0` is released
- **THEN** the GitHub Release notes are the text of that section

#### Scenario: The section is still unreleased

- **WHEN** the section for the version says `unreleased`, or there is no section for it
- **THEN** the release fails and no GitHub Release is created

### Requirement: A release passes all checks first

A GitHub Release SHALL only be created after formatting, lints, the workspace tests with PostgreSQL and the end-to-end
suite have passed on the tagged commit.

#### Scenario: A check fails

- **WHEN** any of these checks fails for a pushed tag
- **THEN** no GitHub Release is created

### Requirement: A released version never changes

A tag with a published GitHub Release SHALL NOT be moved or deleted. A correction SHALL be a new version.

#### Scenario: A released version has a bug

- **WHEN** a bug is found in a published version
- **THEN** the fix is released under a new version number

#### Scenario: A tag failed before it was released

- **WHEN** a tag failed its checks, so it has no GitHub Release
- **THEN** it may be deleted and pushed again after the fix

### Requirement: The version number says what changed

While the major version is 0, a breaking change SHALL raise the minor number and any other change the patch number.
A breaking change is one to the public Rust API, the HTTP wire format, the store schema, a contract hash, or a guarantee
in `docs/architecture.md`.

#### Scenario: A breaking change

- **WHEN** a release from `0.3.1` contains a breaking change
- **THEN** its version is `0.4.0`

#### Scenario: A fix or an addition

- **WHEN** a release from `0.3.1` contains only fixes or additions
- **THEN** its version is `0.3.2`

### Requirement: Another private project can depend on a release

The documentation SHALL say how a project pins a release, and how it gets access to the private repository on a
developer machine, in CI and in a Docker build.

#### Scenario: Pinning a release

- **WHEN** a project follows the documentation
- **THEN** it names the tag once, and every Cicada crate it uses comes from that tag

#### Scenario: Building in the CI of another repository

- **WHEN** a project follows the documentation in GitHub Actions
- **THEN** Cargo can fetch this private repository using a read-only credential of that project
