# Proposal

## Why

The README is the user guide: one file of more than a thousand lines that covers every crate. After ADR 0018 the
library has four layers, and a reader should be able to read only the layer they use. Of the generated references,
only the OpenAPI document is checked for drift. Nothing checks that each guarantee still has a test, nothing checks
a link between the docs, a public item without a doc comment is only a warning, and eleven specs still hold the
placeholder Purpose that archiving wrote.

## What Changes

- `docs/guide/` gets one guide per layer: the core, HTTP and the client, the app, and the ready-made actors. Every
  Rust block is compiled as a doctest and taken from a tested example.
- Generated references are checked for drift by a test: the OpenAPI document (as today), the command line help in
  `docs/reference/cli.md`, and the contracts of the ready-made actors in `docs/reference/contracts/`.
- `docs/guarantees.md` maps each of G1–G6 and S1–S10 to the tests that prove it. A test fails when a test named in
  the table is gone, or a guarantee has no row.
- A test checks the relative links and anchors of the Markdown files of the repository.
- `missing_docs` becomes an error.
- The README shrinks to an overview that points into the guides.
- Every spec in `openspec/specs/` gets a real Purpose. Eleven still hold the placeholder that archiving wrote, so
  `openspec validate --specs --strict` fails for them. A Purpose is edited in the main spec, not through a delta.

The simplest version: plain Markdown in the repository, checked by `cargo test`.

To decide in the design:

- What stays in the README.
- Whether `docs/operations.md` stays one file or joins the guide of the app layer.
- The check of the command line help for drift. The help of a binary depends on the name it was called with and
  on the feature `tui` of `cicada-app`, which is on in a build of the workspace and off in a build of the crate
  alone (found when `cicada-app` was reviewed).

## Capabilities

### New Capabilities

- `documentation`: guides per layer whose code compiles, references that cannot drift, and a table of guarantees
  whose tests exist.

### Modified Capabilities

None.

## Out of scope

- A documentation website.
- Docs per released version.

## Order

Step 11 of ADR 0018, the last one. After `cicada-actors`.

## Impact

- `README.md`, `docs/` (the guides, the references, the table of guarantees), the doctest hooks in
  `examples/build.rs` and `examples/src/lib.rs`, the workspace lints in `Cargo.toml`, a test in `cicada-app` (the
  help) and in `cicada-actors` (the contracts, read from their new place), the eleven specs with a placeholder
  Purpose, `AGENTS.md` (its rules on doctests and doc comments) and `CHANGELOG.md`.
- No change to code behaviour, a guarantee or the wire format; no ADR.
