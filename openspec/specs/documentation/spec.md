# documentation Specification

## Purpose
The docs are one guide per layer, with a README that points into them. Every Rust block compiles, the references
(the command line help, the contracts of the ready-made actors) cannot drift from the code, each guarantee names the
tests that prove it, and links between the docs do not break.

## Requirements

### Requirement: There is one guide per layer

`docs/guide/` SHALL hold a guide for each layer: `core.md` for `cicada`, `http.md` for `cicada-axum` and
`cicada-client`, `app.md` for `cicada-app`, and `actors.md` for `cicada-actors`. The README SHALL be an overview
that names every guide, the architecture, the operations guide and the decisions.

#### Scenario: A reader of one layer

- **WHEN** a reader who uses only `cicada-axum` opens the README
- **THEN** it names the guide of the HTTP layer, and that guide covers the routers, the HTTP API, the schema check,
  mounting in an axum service and the client

#### Scenario: The operations guide

- **WHEN** a reader opens the guide of the app layer
- **THEN** it links to `docs/operations.md` for sizing, deploying, upgrading and on call

### Requirement: Every Rust block of the docs compiles

Every Rust block of the README, of the operations guide and of each guide SHALL be a doctest of `cicada-examples`,
compiled by `cargo test -p cicada-examples --doc`, and run unless it is marked `no_run`.

#### Scenario: A block that no longer compiles

- **WHEN** a public item that a block of `docs/guide/core.md` uses is renamed
- **THEN** `cargo test -p cicada-examples --doc` fails and names that block

#### Scenario: A hidden setup that drifts

- **WHEN** the hidden setup comment of a block says it stands for more lines than the visible block has
- **THEN** the build of `cicada-examples` fails

### Requirement: The help of the command line is a reference that cannot drift

`docs/reference/cli.md` SHALL hold the help of the plain `cicada` binary and of every command and subcommand, and
that of `serve` and `worker`. A test SHALL fail when the file differs from the help the code renders, and SHALL
rewrite it when the tests run with `CICADA_UPDATE_CLI=true`. The help SHALL be rendered with a fixed name and with
the terminal UI.

#### Scenario: A flag is added

- **WHEN** a flag is added to an operator command and the tests run
- **THEN** the test fails and says that `docs/reference/cli.md` is out of date and how to rewrite it

#### Scenario: A subcommand is added

- **WHEN** a subcommand is added and the tests run
- **THEN** the test fails until the reference holds the help of the new subcommand

#### Scenario: The name of the test binary

- **WHEN** the test runs from a binary of another name
- **THEN** the help it compares names `cicada`

### Requirement: The contracts of the ready-made actors are a reference that cannot drift

The contract snapshots of `cicada.supervisor`, `cicada.dead-letters` and `cicada.topic` SHALL be in
`docs/reference/contracts/`, and a test SHALL fail when a contract differs from its snapshot as text, descriptions
included.

#### Scenario: A field of a ready-made actor changes

- **WHEN** a field of the state of the supervisor is renamed and the tests run
- **THEN** the test fails and names what changed in the contract of `cicada.supervisor`

#### Scenario: Only a doc comment changes

- **WHEN** the doc comment of a message of the topic is reworded and the tests run
- **THEN** the test fails, and `CICADA_UPDATE_CONTRACTS=true` rewrites the snapshot with the new text

### Requirement: Each guarantee names the tests that prove it

`docs/guarantees.md` SHALL have a row for each of G1–G6 and S1–S10, and each row SHALL name at least one test as
`path::test_name`, `path` being a file from the root of the repository. A test SHALL fail when one of G1–G6 and
S1–S10 or another guarantee of the architecture has no row, when a row names no test, or when a named file holds no
function of that name.

#### Scenario: A named test is gone

- **WHEN** a test that the row of S5 names is renamed
- **THEN** the test of the table fails and names S5 and the missing test

#### Scenario: A table that cannot be read

- **WHEN** the table of guarantees is in a form the test cannot read
- **THEN** the test fails, because G1–G6 and S1–S10 have no row

#### Scenario: A guarantee without a row

- **WHEN** the architecture gets a guarantee S11 and the table no row for it
- **THEN** the test of the table fails and names S11

### Requirement: Links between the docs do not break

A test SHALL fail for a relative link in a Markdown file of the repository (outside `target` and the archive of
changes) whose file does not exist, or whose anchor names no heading of that file.

#### Scenario: A section moves

- **WHEN** a section of the README that `docs/operations.md` links to by its anchor is moved into a guide
- **THEN** the test fails and names the link, until the link points into the guide

#### Scenario: A link to the web

- **WHEN** a Markdown file links to a page on the web
- **THEN** the test does not check it

### Requirement: A public item without a doc comment is an error

The workspace lints SHALL make `missing_docs` an error.

#### Scenario: A public function without a doc comment

- **WHEN** a public function without a doc comment is added to a crate of the workspace
- **THEN** `cargo build` fails

### Requirement: Every spec has a real Purpose

No spec in `openspec/specs/` SHALL hold the placeholder Purpose that archiving writes.

#### Scenario: The strict check of the specs

- **WHEN** `openspec validate --specs --strict` runs
- **THEN** it passes for every spec

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
