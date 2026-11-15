## MODIFIED Requirements

### Requirement: A release passes all checks first

A GitHub Release SHALL only be published after valid local checks and E2E evidence for its pinned commit, matching
workspace/skill versions, a dated changelog section and verified skill packaging. Full checks SHALL preserve the
existing CI coverage: Python tests, formatting, warning-free lints, workspace tests with PostgreSQL, isolated-feature
builds with warnings denied, app-alone tests and dependency-tree checks. Docker E2E SHALL run without a test filter.
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

### Requirement: A released version never changes

A published GitHub Release SHALL NOT be changed by the release command. Every remote version tag SHALL be immutable
under the gate, even before publication. A correction SHALL use a new version; publication failures after a verified
tag push SHALL resume against the same tag object, not delete or move it.

#### Scenario: A released version has a bug

- **WHEN** a bug is found in a published version
- **THEN** the fix is released under a new version number

#### Scenario: Publication failed after a tag push

- **WHEN** a verified version tag exists remotely but the GitHub API step failed
- **THEN** a retry verifies the same raw tag object and commit before continuing
- **AND** the gate refuses tag deletion or replacement, including an annotated/lightweight substitution at the same commit

#### Scenario: A tag failed before it was released

- **WHEN** an unpublished remote version tag exists and checks for it fail
- **THEN** the gate refuses publication and also refuses deleting or replacing that remote tag
- **AND** a correction uses a new version; the earlier permission to delete failed unpublished tags no longer applies

## ADDED Requirements

### Requirement: Local evidence names the checked Git content and policy

The gate SHALL check a pinned commit's exact tree with its supported policy and matching runner in an owned detached
checkout. Only completed passing evidence SHALL authorize a push. Records SHALL be shared in Git's common directory
and atomically replaced. No expiry or unrecorded toolchain equivalence SHALL be claimed.

#### Scenario: Recording a completed run

- **WHEN** all commands finish
- **THEN** the record names commit/tree, policy/runner blobs, kind, exact commands and results, toolchain and timestamps
- **AND** derived docs evidence names its base; unsupported policy or runner mismatch refuses a run

#### Scenario: Shared worktree has other edits

- **WHEN** the caller has dirty, untracked or ignored files unrelated to the pinned commit
- **THEN** those files are neither tested nor modified by the gate
- **AND** evidence names the isolated checked commit and tree

#### Scenario: A check changes its source

- **WHEN** a check changes HEAD, tracked files, or untracked/ignored files outside the owned target cache
- **THEN** the run fails and writes no passing evidence

#### Scenario: Missing or inconsistent evidence

- **WHEN** a record has an unknown schema, wrong object/policy/runner, missing command, nonzero exit or invalid base
- **THEN** it does not authorize a push or publication
- **AND** status distinguishes missing, running and failed evidence from a pass

#### Scenario: Forced run fails or is interrupted

- **WHEN** a passing tree is rerun with force
- **THEN** the old record becomes running before execution and a failure becomes failed
- **AND** neither failure nor interruption falls back to that old pass

### Requirement: Reuse is limited to audited text inputs with fresh checks

A direct full pass MAY serve identical trees. Otherwise fresh docs checks SHALL require a direct-full eligible
ancestor and an allowlisted text-only cumulative diff. E2E SHALL pass directly or reuse the same eligible direct-full
base's direct E2E pass. Derived records SHALL NOT form chains. Every use SHALL revalidate eligibility; failed/running
target attempts SHALL block fallback until rerun successfully. Notes and packaging SHALL always be verified anew.

#### Scenario: Base and diff eligibility

- **WHEN** the gate searches for a docs base
- **THEN** it checks at most 100 first-parent ancestors, nearest first, requiring a direct full pass and identical policy/runner
- **AND** it classifies the raw NUL-delimited cumulative diff with rename detection off
- **AND** only regular non-executable additions/modifications/deletions qualify under README.md, CHANGELOG.md,
  AGENTS.md, docs/**/*.md excluding docs/reference/**, skills/cicada/**/*.md, .claude/skills/**/*.md and openspec/**/*.md

#### Scenario: Fresh proof and failed attempts

- **WHEN** the target qualifies for docs checks
- **THEN** Python and examples doctest/docs/skill checks run on its exact tree
- **AND** any running/failed target full or docs record blocks checks fallback until that kind is successfully rerun
- **AND** a running/failed target E2E record similarly blocks E2E fallback

#### Scenario: Documentation changes after a full pass

- **WHEN** only allowed Markdown differs from the direct-full base
- **THEN** fresh docs checks on the target and a revalidated base authorize checks reuse
- **AND** failing doctests, links, guarantee rows or skill routes refuse it

#### Scenario: Non-documentation input or file type changes

- **WHEN** a change affects docs/reference, a crate README, OpenSpec YAML, unknown files, executable modes, a symlink
  or a submodule, including either side of a rename
- **THEN** the docs shortcut is refused and full checks are required

#### Scenario: No valid direct base

- **WHEN** only a derived docs record exists or the direct base lies outside the search window
- **THEN** the gate runs full checks instead of following a chain

### Requirement: Gate execution owns its resources and fails closed on interruption

Run and release SHALL serialize owned-worktree mutation with an inheritable flock and an in-progress marker.
Busy/unsupported locks and abandoned markers SHALL refuse destructive reuse. Only a verified owned worktree MAY
be cleaned. Child groups SHALL be reaped on normal cancellation. Checks SHALL use controlled environment inputs;
gate metadata SHALL omit secrets. Other worktrees SHALL remain untouched.

#### Scenario: Owned execution environment

- **WHEN** a run prepares its worktree
- **THEN** registration, matching common directory and gate ownership marker are required before destructive steps
- **AND** external ancestor or CARGO_HOME Cargo configuration and ancestor toolchain files are refused
- **AND** the command environment uses the documented allowlist and controlled PostgreSQL/full-suite settings

#### Scenario: Parent dies during a command

- **WHEN** the gate is killed while a child command is alive
- **THEN** the inherited lock prevents another run from cleaning the checkout
- **AND** no pass is written, and an abandoned marker prevents automatic reuse after the lock becomes free

#### Scenario: Normal cancellation

- **WHEN** the gate receives INT or TERM
- **THEN** it terminates and reaps its command group and records no pass
- **AND** unfinished or uncertain descendants are not declared safely recovered from the parent's exit alone

#### Scenario: Abandoned-marker recovery

- **WHEN** an operator recovers an interrupted run
- **THEN** it holds the lock and verifies recorded process identities/groups and other users of the owned checkout
- **AND** it clears the marker only after identified gate work has ended and no users remain; uncertainty blocks cleanup
- **AND** failed/running evidence remains unusable until a new successful run

#### Scenario: Unowned path

- **WHEN** the gate path already exists without matching worktree registration and ownership marker
- **THEN** the command refuses it without deleting or cleaning that path

#### Scenario: Ambient filters or update flags

- **WHEN** the caller sets E2E_ONLY, update variables, property-test overrides or compiler flags/wrappers
- **THEN** check commands use the controlled environment and complete policy rather than those overrides
- **AND** recorded metadata has no PG credentials, credential query fields or arbitrary environment dump

### Requirement: The installed pre-push gate validates every protected ref update

The hook SHALL inspect all ref updates without network or the execution lock. Main SHALL require fast-forward tip
evidence; new version tags SHALL require checks, E2E and main ancestry. Main deletion and existing remote version-tag
mutation SHALL be refused. Other refs SHALL pass. Installation SHALL respect core.hooksPath, preserve docs hooks and
refuse overwriting a foreign pre-push.

#### Scenario: Version tag qualification

- **WHEN** the destination is a strict vX.Y.Z tag
- **THEN** a new tag's peeled commit needs checks and E2E and ancestry of previously accepted main in the known
  local tracking ref of that named remote
- **AND** an existing remote tag cannot be updated or deleted

#### Scenario: Main needs evidence

- **WHEN** a push targets main through an ordinary ref, HEAD:main or a combined push
- **THEN** the same tip evidence and fast-forward rules apply
- **AND** missing evidence, unknown old OID, deletion or a force/non-fast-forward update is refused

#### Scenario: Tag depends on an unaccepted main update

- **WHEN** a new version tag and main update arrive in either stdin order
- **THEN** proposed main ancestry does not qualify the tag; it must already belong to previously accepted main
- **AND** a tag depending only on that update refuses the push before the server can partially accept it

#### Scenario: Server rejects only main

- **WHEN** a server would reject main but accept tags in an ordinary combined push
- **THEN** the local hook refuses a tag outside previously accepted main even if its candidate main has check evidence
- **AND** the tested server receives neither update; the release driver instead pushes main and verifies it before the tag

#### Scenario: Foreign hook or unknown remote tracking

- **WHEN** another hook owns pre-push or a tag-only push has no usable named-remote main tracking ref
- **THEN** installation refuses with chaining guidance or the push refuses with fetch guidance, respectively
- **AND** neither operation guesses origin for a different or URL-only remote

### Requirement: Local release pins and revalidates publication state

Only the owner-approved build loop SHALL publish. Release SHALL pin commit C and raw tag object O, verify required
evidence, notes and package, and publish without force to origin's main/tag and repository. Stored stages SHALL be
recovery hints, never substitutes for current remote checks. Caller refs/files SHALL not be reset. Conflicting
remote state SHALL stop publication rather than be overwritten.

#### Scenario: First invocation and retry

- **WHEN** release starts or resumes
- **THEN** it persists tag/C and later O, defaults retries to stored C and rejects an explicit different C
- **AND** exact version/notes preflight, checks and E2E precede the main push; full ancestry verification follows
- **AND** verified package/SOURCE identity precedes the raw tag-object push

#### Scenario: Caller or remote main moves

- **WHEN** HEAD changes after C is pinned or remote main advances during publication
- **THEN** only C can be pushed/tagged/packaged
- **AND** containing remote main is accepted, a rejected race is refreshed once, and unresolved divergence stops

#### Scenario: Tag object or package mismatch

- **WHEN** local or remote tag identity changes or packaged SOURCE/bytes do not match C
- **THEN** publication stops without overwriting a remote tag
- **AND** package failure before the initial tag push leaves no new remote version tag

#### Scenario: Main was pushed but publication failed

- **WHEN** a later version, package or GitHub step fails after main publication
- **THEN** main is left intact and the command reports the completed stage
- **AND** retry rechecks evidence, refs and artifacts rather than rewinding main

#### Scenario: Matching draft resumes

- **WHEN** an existing draft has the expected tag, title, exact notes and only the expected skill asset with matching hash
- **THEN** retry uploads a missing asset if needed, verifies the draft and publishes it
- **AND** unexpected content, extra assets, multiple releases or a conflicting hash stops without clobbering

#### Scenario: Existing published Release

- **WHEN** a published Release already exists for the expected remote tag
- **THEN** the command leaves it untouched and reports already published, without claiming new historical verification

#### Scenario: Gate rollout

- **WHEN** the owner approves landing this change
- **THEN** the build loop installs the local hook and releases using the checked new implementation
- **AND** its pushed tree already disables automatic Actions, with no interim hook bypass or unchecked publication

#### Scenario: Local enforcement limits

- **WHEN** the workflow is documented
- **THEN** it states that another clone, old hook, forged local record or --no-verify can bypass this local control
- **AND** it does not claim server enforcement, hermetic builds, Linux coverage from macOS or atomic Git/GitHub publication
