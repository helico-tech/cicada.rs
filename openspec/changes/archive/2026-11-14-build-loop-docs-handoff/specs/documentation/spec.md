## ADDED Requirements

### Requirement: A built change hands user-facing prose to the docs session

Every built change SHALL include docs-handoff.md with version, user-facing source changes, proving tests/examples,
destination files and heading text, proposed substance, caveats, blocking checks or none, and actual verification/docs
status. No-impact changes SHALL state None and why. The builder SHALL update it with behavior-changing fixes; review
SHALL compare it against the request's exact head. Landing SHALL report the archived handoff for docs maintenance.

#### Scenario: Additive change

- **WHEN** a source change needs new guidance but all existing docs and skill checks pass
- **THEN** the builder supplies the complete handoff with no blocking items and runs the ordinary finishing checks
- **AND** review checks its accuracy and Present distinguishes handed-off prose from prose already written

#### Scenario: No user impact or release-only guidance

- **WHEN** a change has no user-facing behavior
- **THEN** its handoff states None with a reason, rather than omitting the file
- **AND** its version still reaches the docs session for installation-guide pin maintenance

#### Scenario: Revised implementation

- **WHEN** a fix round or rebase changes user-visible behavior
- **THEN** the handoff changes with it and review checks the current request head
- **AND** it need not contain a circular hash of its own commit; archived reviews and landed source identify the evidence

### Requirement: Documentation ownership remains unchanged by the handoff

Builders SHALL leave protected prose, including embedded Rust blocks, to the docs owner. They SHALL retain source,
rustdoc, example code, skill routes/version, ADR, generated reference and changelog responsibilities. Existing tasks
requesting protected edits SHALL be fulfilled through handoff without dropping their checks. README release pins
SHALL retain their existing exception; install-guide pins SHALL stay with docs. No hook bypass SHALL be introduced.

#### Scenario: Earlier task names a guide edit

- **WHEN** an existing ready task asks the builder to edit a layer guide or guarantees table
- **THEN** the builder supplies the needed content in its handoff and coordinates owner preparation if checks depend on it
- **AND** it does not independently edit the protected file, even temporarily

#### Scenario: Version and routes

- **WHEN** a release or new example changes required versions/routes
- **THEN** the producer keeps skill metadata/routes and allowed generated artifacts current
- **AND** install-guide updates are handed to docs, while the release loop updates only the allowed README tag substitutions

### Requirement: No change lands with a known failing documentation check

Required doctests, links, guarantee references and consumer-skill checks SHALL pass before normal completed-build
review and landing. A builder blocked on protected content SHALL finish independent work, leave affected tasks
unticked and report concrete failed checks and replacements. A handoff SHALL NOT excuse a failing check or claim
unwritten prose is complete. This requirement SHALL be verified by workflow review and existing checks, not prose-mirroring tests.

#### Scenario: Breaking rename

- **WHEN** a public API rename breaks a protected doc block
- **THEN** the builder reports that block/check as blocking and supplies a proposed replacement through the handoff
- **AND** it does not disable the block, weaken the check or land broken source pending a later docs scan

#### Scenario: Docs owner unavailable

- **WHEN** no owner-prepared fix is available
- **THEN** independent work is retained and the build remains blocked with affected checks unticked
- **AND** elapsed time or an accurate blocker report does not authorize bypass or a code-review pass

### Requirement: Blocking fixes are integrated as exact owner-prepared content

The docs owner SHALL prepare blocking fixes against the identified feature head and current base. The builder MAY
integrate agreed exact blobs/deletions without merging a branch, provided they match the current recorded owner branch
at commit and preserve unrelated changes. Required fixes SHALL be combined with breaking code before finishing checks,
normal review and the owner's existing land/release approval. No intermediate known docs break SHALL be introduced to main.

#### Scenario: Prepared fix

- **WHEN** the owner provides a prepared commit, source head and affected paths including rename/deletion sides
- **THEN** the builder inspects the complete diff, integrates only those paths and verifies current owner-branch identity
- **AND** it reruns blocked and finishing checks on the combined result before reporting completion and requesting review

#### Scenario: Changed source or prepared content

- **WHEN** a fix round/rebase invalidates prepared prose or a relevant owner-branch blob changes
- **THEN** the builder obtains refreshed owner preparation and rechecks identity/compatibility
- **AND** a conflict-free late rebase still requires compatibility and affected-check verification
- **AND** a protected-file conflict is not resolved by blindly selecting one side or overwriting unrelated prose

#### Scenario: Coordination before automatic adoption

- **WHEN** the docs session has not adopted discovery of unpublished blocking handoffs
- **THEN** the build loop identifies the concrete head and handoff for explicit owner coordination and keeps the build blocked
- **AND** this change neither claims a feature-branch scanner exists nor publishes code-dependent prose separately before source

#### Scenario: Review and release authority

- **WHEN** all combined checks pass and the current head has normal review
- **THEN** Present reports the integrated required fixes separately from remaining asynchronous guidance
- **AND** only the existing owner-approved build loop lands/releases; builders and reviewers gain no merge/push/tag authority
