## MODIFIED Requirements

### Requirement: Local release pins and revalidates publication state

Only the owner-approved build loop SHALL publish. Release SHALL pin commit C and raw tag object O, verify required
evidence, notes and both packages, and publish without force to origin's main/tag and repository. Stored stages SHALL be
recovery hints, never substitutes for current remote checks. Caller refs/files SHALL not be reset. Conflicting
remote state SHALL stop publication rather than be overwritten.

#### Scenario: First invocation and retry

- **WHEN** release starts or resumes
- **THEN** it persists tag/C and later O, defaults retries to stored C and rejects an explicit different C
- **AND** exact version/notes preflight, checks and E2E precede the main push; full ancestry verification follows
- **AND** verified package/SOURCE identity and the saved digest map for both archives precede the raw tag-object push

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

- **WHEN** an existing draft has the expected tag, title, exact notes and any subset of the expected skill and
  fixture assets, with each name present at most once and every present digest matching
- **THEN** retry uploads only missing assets without clobber, re-reads the draft after each upload including an
  uncertain failure, and publishes only after both expected assets are verified
- **AND** unexpected content, duplicate or extra assets, multiple releases or a conflicting hash stops without clobbering

#### Scenario: Saved asset identity and legacy state

- **WHEN** both packages have been checked for a new release
- **THEN** their complete filename-to-digest map is persisted atomically
- **WHEN** packaging on retry changes a digest or the expected name set
- **THEN** the release refuses rather than replacing the saved map
- **WHEN** the new driver encounters a legacy singular `asset_sha256` field
- **THEN** it refuses with a diagnostic to resume the release with its pinned runner, without converting that state
- **AND** a pre-package state with neither digest representation may proceed, and evidence-record schema is unchanged

#### Scenario: Existing published Release

- **WHEN** a published Release already exists for the expected remote tag
- **THEN** the command leaves it untouched and reports already published, without claiming new historical verification
- **AND** older published releases are not backfilled with a fixture archive

#### Scenario: Gate rollout

- **WHEN** the owner approves landing this change
- **THEN** the build loop installs the local hook and releases using the checked new implementation
- **AND** its pushed tree already disables automatic Actions, with no interim hook bypass or unchecked publication

#### Scenario: Local enforcement limits

- **WHEN** the workflow is documented
- **THEN** it states that another clone, old hook, forged local record or --no-verify can bypass this local control
- **AND** it does not claim server enforcement, hermetic builds, Linux coverage from macOS or atomic Git/GitHub publication
