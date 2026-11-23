## MODIFIED Requirements

### Requirement: A release passes all checks first

A GitHub Release SHALL only be published after valid local checks and E2E evidence for its pinned commit, matching
workspace/skill versions, a dated changelog section and verified skill packaging. Full checks SHALL preserve the
existing CI coverage: Python tests, formatting, warning-free lints, workspace tests with PostgreSQL, isolated-feature
builds with warnings denied, app-alone tests and dependency-tree checks. Full checks SHALL also check
`Cargo.lock` against the RustSec advisory database with `cargo deny check advisories`; `deny.toml` SHALL name each
accepted advisory with its reason. Docker E2E SHALL run without a test filter.
Documentation-only reuse SHALL follow the explicit evidence rules below, not skip validation. Automatic push, pull
request and tag Actions SHALL be removed; optional manual Linux CI SHALL run the same full policy without publishing.

#### Scenario: A check fails

- **WHEN** a required check, version check or package verification fails
- **THEN** the command fails and no new version tag or GitHub Release is published by that attempt
- **AND** a tag already pushed by an earlier successful verification is retained for recovery, never rewritten

#### Scenario: Coverage moves locally

- **WHEN** a full check run replaces CI
- **THEN** every existing feature, driver, TLS, crypto-provider, terminal-library and app-alone check remains
- **AND** PostgreSQL being unset or unreachable cannot produce passing evidence through skipped tests

#### Scenario: Manual Linux check

- **WHEN** a maintainer explicitly starts the manual workflow
- **THEN** it runs the common full check policy with PostgreSQL and creates neither local evidence nor a Release
- **AND** ordinary main, pull-request and version-tag updates start no automatic workflow

#### Scenario: A dependency has an advisory

- **WHEN** a crate in `Cargo.lock` has a RustSec advisory that `deny.toml` does not accept
- **THEN** the full checks fail and give no evidence for the commit
- **AND** the advisory database is fetched for each run, so a run without it fails instead of passing
