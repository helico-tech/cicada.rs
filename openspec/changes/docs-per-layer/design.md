# Design

Described on 2026-11-12 (change loop, Describe), while `cicada-actors` is built. The owner handed the decisions of
this change to the main session on 2026-11-11 ("Dor change 5 until 9 i fully trust you"). Every decision below is
made by the main session; each gives the options and why. Reviewed once; what changed after the review is marked
"review".

## Context

What the repository has today (release 0.13.0, with `cicada-actors` of 0.14.0 being built):

- `README.md` is the user guide: about 1,600 lines, every crate and layer in one file. `docs/architecture.md` (about
  260 lines) is the reference for behaviour, with the tables of G1–G6 and S1–S10. `docs/operations.md` (about 480
  lines) is the guide for operators: sizing, deploying, upgrading, releasing, on call, store tuning, telemetry.
- The Rust blocks of `README.md` and `docs/operations.md` are doctests of `cicada-examples`: `examples/build.rs`
  turns each file into rustdoc markdown, where a hidden `<!-- doctest … -->` comment can add setup lines and `@@N`
  stands for the next N lines of the visible block, so the two cannot drift apart. `ReadmeDoctests` and
  `OperationsDoctests` in `examples/src/lib.rs` include them.
- The examples crate holds ten worked examples, each with tests.
- `docs/openapi.json` is checked against the generated document by a test of `cicada-axum`, and rewritten when the
  tests run with `CICADA_UPDATE_OPENAPI=true`. Contract snapshots are checked by
  `cicada_test::assert_contracts_match` and rewritten with `CICADA_UPDATE_CONTRACTS=true`; `cicada-actors` has
  snapshots of its three types.
- Nothing generates or checks the help of the command line. The help names the binary as it was called, and the
  command `tui` exists only with the feature `tui` of `cicada-app`, which a build of the workspace turns on and a
  build of the crate alone does not.
- `missing_docs` is `warn` in the workspace lints. CI runs clippy with `-D warnings`, so a public item without a doc
  comment already fails CI, but not `cargo build` or `cargo test`.
- Eleven specs in `openspec/specs/` still hold the placeholder Purpose that archiving wrote, so
  `openspec validate --specs --strict` fails for them.
- Other files point into the README: `docs/architecture.md` ("See the README for the endpoints"),
  `docs/operations.md` (seven places, one with an anchor), `AGENTS.md` ("README.md is the user guide", the doctest
  lines) and the rule of `openspec/config.yaml` on doctests. Nothing checks a link in the docs.
- `CICADA_UPDATE_CONTRACTS=true` rewrites a snapshot only when its hash differs, and the hash leaves out the
  descriptions (ADR 0017), so a snapshot can keep an old doc text.
- The README has a table of the ADRs, to which each change adds a row.

ADR 0018: "One guide per layer in `docs/guide/`, with every Rust block compiled as a doctest and taken from tested
examples. Generated references (OpenAPI, command line help, the contracts of the ready-made actors) are checked for
drift by a test. A table maps each of G1–G6 and S1–S10 to the tests that prove it, and a test fails when a named test
is gone. `missing_docs` is an error."

## Goals / Non-Goals

**Goals**

- A reader reads only the layer they use.
- Nothing in the docs that a test can check is left unchecked: code, references, links between the docs, and the
  tests behind each guarantee.

**Non-Goals**

- A documentation website, or docs per released version.
- Rewriting `docs/architecture.md`: it stays the reference for behaviour.
- Changing code behaviour, a guarantee or the wire format.

## Decisions

### 1. Four guides, one per layer

| Guide | Layer | What it covers |
| --- | --- | --- |
| `docs/guide/core.md` | 0, `cicada` | opening a store and its features, `NodeConfig`, actors, messages, turns, state, child actors, watches, control effects, retry, retention, the claim limit, observers, testing with `TestActors`, contract snapshots and state samples, the worked examples |
| `docs/guide/http.md` | 1, `cicada-axum`, `cicada-client` | the routers, the HTTP API, the schema check, mounting in an axum service, the client |
| `docs/guide/app.md` | 2, `cicada-app` | `App`, `serve` and `worker`, the settings, the command line and the terminal UI, the sample service |
| `docs/guide/actors.md` | 3, `cicada-actors` | the supervisor, dead letters, the topic, `Schedule` |

- The text of each guide comes from the README of today, moved and put in order, not written again. What is new is
  the order and the links between the guides.
- `cicada-otel` is a section of the core guide (an observer), and `cicada-test` one of the core guide (testing).

### 2. Every Rust block of every guide is a doctest

`examples/build.rs` turns the four guides into rustdoc markdown as it does the README and the operations guide, and
`examples/src/lib.rs` gets one doctest struct per guide. The hidden setup comment and `@@N` work as today.

"Taken from tested examples" (ADR 0018) is read narrowly: the blocks are tested themselves. A block that does not
compile, or that fails when it runs, fails `cargo test`; a `no_run` block (one that starts a server or needs a
database) only compiles. A block that shows a worked example names the file of `examples/` it comes from.

Not chosen: blocks included from the example files at build time. The examples are written to be tested, not read
in pieces; cutting them into blocks would need markers in the code and a second tool.

### 3. What stays in the README

The README becomes an overview of about 200 lines:

- what Cicada is, in a paragraph, and the layers with their crates in a table;
- a quick start: one actor type, a `main` on `App`, and the commands to run it and send it a message;
- how to use a release in another project (the tag lines that the release steps update);
- a table of the guides, the architecture, the operations guide and the decisions;
- building and testing, briefly, with a link to `AGENTS.md`.

Its Rust blocks stay doctests. Where the rest goes (review):

| Today in the README | After |
| --- | --- |
| the sections on the core, HTTP, the command line, testing, retry, the claim limit, persistence, observers, the extras | the guide of their layer (decision 1) |
| "Configuration" | `NodeConfig` in the core guide, the flags and variables in the app guide |
| "Examples" | the core guide |
| "Sample service" | the app guide |
| "Decisions", the table of the ADRs | `docs/adr/README.md`; a change adds its ADR's row there from now on |
| the design notes and the known limitations | `docs/architecture.md` |

### 4. The operations guide stays one file

`docs/operations.md` is for operators, who run a system and do not read by layer. It stays one file, and the app
guide links to it. Its pointers into the README are changed to point into the guides (decision 8).

Not chosen: joining it with the app guide. Sizing, upgrading and on call are about the whole system, and the store
and the core appear in them as much as the app.

### 5. The help of the command line is a reference

`docs/reference/cli.md` holds the help of the plain `cicada` binary: the help of the binary and of every command and
subcommand, each under a heading. A unit test inside `cicada-app` renders it: it walks the tree of the command line
with clap's `get_subcommands()`, so a new subcommand is in the reference without a list to keep (review; a test in
`tests/` cannot reach the tree, which is private to the crate). The test fails when the file differs, and rewrites
it with `CICADA_UPDATE_CLI=true`, as the OpenAPI test does with `CICADA_UPDATE_OPENAPI`.

- **The name:** the test renders the help with the name `cicada`, so it does not depend on how the test binary is
  called.
- **The feature `tui`:** the reference holds the help with the terminal UI. The test runs only when the feature is
  on (`#[cfg(feature = "tui")]`), which it is in a build of the workspace, where `cargo test --workspace` runs it.
  `cargo test -p cicada-app` alone skips it. The docs of the variable say to run it with `--workspace` or
  `--features tui`.
- **The operator commands only:** the plain binary has no `serve` or `worker` (ADR 0027). Their help is the same in
  every host's binary but for the name; the reference holds it once more under a heading of its own, rendered from
  an `App` with one test type and the name `myapp`.

Not chosen: two references, with and without `tui`. One more file for a difference of one command.

`docs/openapi.json` stays where it is, outside `docs/reference/`: tools and links outside the repository point at
it, and moving it buys nothing.

### 6. The contracts of the ready-made actors are a reference

The contract snapshots of `cicada-actors` move to `docs/reference/contracts/`, one file per type, and its test reads
them there. They are what a host depends on, so they belong with the other references.

The test compares each file with the contract as pretty JSON text, descriptions included, not by its hash (review):
a reader of a reference reads the doc text, and a comparison by hash would let it lag. `CICADA_UPDATE_CONTRACTS=true`
rewrites a file that differs in any way.

### 7. A table of the tests behind each guarantee

`docs/guarantees.md` has one row for each of G1–G6 and S1–S10: the name of the guarantee, one line of what it says
(the architecture has the full text), and the tests that prove it, each as `path::test_name`, where `path` is the
path of a Rust file from the root of the repository. A test of `cicada-examples` reads the table and fails when:

- one of G1, …, G6, S1, …, S10 has no row (the list is in the test, so a table that cannot be read fails it), a
  guarantee that the architecture names has no row, or a row names no test;
- a named file does not exist, or holds no `fn test_name`.

The conformance cases are named by their function in `crates/cicada/src/tests/conformance`, which runs them on
every store; a row names the store-independent case, not each store's copy.

- **A file of its own (chosen).** The architecture stays a text about behaviour, and the table can be long.
- A column in the tables of the architecture. The rows would become hard to read, and every change of a test would
  touch the reference for behaviour.

The test finds a test by its name in its file. It does not run it; `cargo test` does that.

### 8. Links between the docs are checked

The same test of `cicada-examples` reads every Markdown file of the repository outside `target` and
`openspec/changes/archive`, and fails for a relative link whose file does not exist, or whose anchor names no
heading of that file. Links to the web are not checked. That is what this change breaks most easily: the README
loses its sections, and `docs/operations.md`, `docs/architecture.md`, `AGENTS.md` and the specs point into it.

The files whose pointers change: `docs/architecture.md`, `docs/operations.md`, `AGENTS.md`, `openspec/config.yaml`
(its rule on doctests names the guides), and the proposal of `value-depth-limit`. The file
`.claude/skills/change-loop/validator.md` names the doctests of the README and the operations guide; it stays as it
is (the owner said not to change the skill files for now), and it is still true.

The Rust block of `docs/architecture.md` stays a sketch that is not compiled; the architecture is a reference, not a
guide.

### 9. `missing_docs` is an error

The workspace lints say `missing_docs = "deny"`. Every public item has a doc comment today (CI already fails on the
warning), so nothing else changes; a missing one now also fails `cargo build`.

### 10. Every spec has a real Purpose

The eleven specs with the placeholder get a Purpose of one to three sentences, written in the main specs, not
through a delta: a Purpose is not a requirement. `openspec validate --specs --strict` passes after.

### 11. `AGENTS.md`

Its map and its rules name the guides instead of "the README is the user guide", the doctests of the guides, the
references and their variables (`CICADA_UPDATE_OPENAPI`, `CICADA_UPDATE_CLI`, `CICADA_UPDATE_CONTRACTS`), the table
of guarantees and the index of the ADRs: a change that adds a guarantee's test adds it to the table; a change to the
help or a ready-made contract regenerates its reference; a new ADR gets its row in `docs/adr/README.md`.

## Risks / Trade-offs

- The README loses most of its text; a reader who knew where something was must follow a link. The overview names
  every guide.
- The reference of the help changes with every change to a command or a flag; a test says when, and one variable
  rewrites it.
- The table of guarantees can name a test that no longer proves its guarantee: the test checks that the test is
  there, not what it does. A review of a change that touches a guarantee checks the table.
- `cargo test -p cicada-app` alone does not check the help reference (decision 5).
- The link check knows relative links and anchors only; a link to the web can break unseen.

## Migration Plan

Version 0.14.1. No change of code behaviour, a guarantee or the wire format. Nothing to do for a host.

## Open Questions

None.
