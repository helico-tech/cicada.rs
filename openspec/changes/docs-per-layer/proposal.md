# Proposal

## Why

The README is the user guide: one file of more than a thousand lines that covers every crate. After ADR 0018 the
library has four layers, and a reader should be able to read only the layer they use. Of the generated references,
only the OpenAPI document is checked for drift. Nothing checks that each guarantee still has a test, and a public item
without a doc comment is only a warning.

## What Changes

- `docs/guide/` gets one guide per layer: the core, HTTP and the client, the app, and the ready-made actors. Every
  Rust block is compiled as a doctest and taken from a tested example.
- Generated references are checked for drift by a test: the OpenAPI document (as today), the command line help, and
  the contracts of the ready-made actors.
- A table maps each of G1–G6 and S1–S10 to the tests that prove it. A test fails when a test named in the table is
  gone.
- `missing_docs` becomes an error.
- The README shrinks to an overview that points into the guides.
- Every spec in `openspec/specs/` gets a real Purpose. Eleven still hold the placeholder that archiving wrote, so
  `openspec validate --specs --strict` fails for them. A Purpose is edited in the main spec, not through a delta.

The simplest version: plain Markdown in the repository, checked by `cargo test`.

To decide in the design:

- What stays in the README.
- Whether `docs/operations.md` stays one file or joins the guide of the app layer.

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

- `README.md`, `docs/`, the doctest hooks in `examples/src/lib.rs`, the workspace lints in `Cargo.toml`, `AGENTS.md`
  (its rules on doctests and doc comments) and CI.
- No change to code behaviour, a guarantee or the wire format; no ADR.
