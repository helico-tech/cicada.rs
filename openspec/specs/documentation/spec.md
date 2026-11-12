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
