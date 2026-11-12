# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `docs-per-layer` has landed when this is built (0.14.1).

## 1. The schemas

- [x] 1.1 Reading a reference [1, 2]: one function in `crates/cicada/src/schema.rs` that reads a `$ref` starting with
      `#/$defs/` as RFC 6901 says for a fragment (percent-decode, split at `/`, unescape `~1` and `~0`) and gives the
      first segment as the name of the definition, used where the definitions of a message are collected and where a
      newtype variant is replaced. A newtype variant whose definition is missing names the variant and the
      definition. Verify: unit tests of the function with names that schemars escapes (a space, `<`, `>`, `/`, `~`,
      a letter that is not ASCII, a `%` that does not decode) and a pointer of several segments; tests for the
      scenarios "A renamed type", "A type renamed for serde", "A type name with a letter that is not ASCII", "A
      newtype variant of a renamed type", "A newtype variant whose definition is missing" and "A reference of several
      segments".
- [x] 1.2 A reference to the family [3]: in each message document, every `"$ref": "#"` under a keyword that holds a
      schema becomes `true`; data in `const`, `enum`, `default` and `examples` stays. Verify: tests for the scenarios
      "The other messages of the family" and "A `#` that is data"; and through the HTTP door, on a node that hosts the
      type and one that does not, "A batch of messages", "A nested message that is wrong, on a node that hosts the
      type" and "… on a node that does not host the type".
- [x] 1.3 Nothing else changes [5]: the contract snapshots of the repository, and a test that the contracts of the
      examples, the ready-made actors and the sample service are byte for byte the same as at 0.14.1. Verify: no
      snapshot changes; the scenario "A type that is not hit".

## 2. Docs

- [x] 2.1 The core guide (the contract of a type: what is hit and how to tell), `docs/operations.md` (the upgrade to
      0.15.0 by the Migration Plan of `design.md`). Verify: `cargo test -p cicada-examples --doc` passes, the link
      check passes, and every risk of `design.md` that names the docs is in them.
- [x] 2.2 `CHANGELOG.md`, a section `0.15.0 — unreleased`, and the version `0.15.0` in `Cargo.toml`: the steps of the
      Migration Plan first, then the breaking change of the proposal, naming the three kinds of type that are hit and
      that a hit newtype variant could not start before. Verify: every step of the plan and every **BREAKING** line of
      the proposal is in the section.

## 3. Checks

- [x] 3.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, `cargo check -p cicada` with no feature and with each feature alone,
      `openspec validate schema-reference-names --strict` and `openspec validate --specs --strict`. Verify: all pass.
- [x] 3.2 The PostgreSQL tests with `CICADA_PG_URL` set, once more against a throwaway `postgres:17.5-alpine` with
      its default 100 connections (removed afterwards), and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. All need Docker. Verify: all pass; if Docker cannot be
      started, leave this task open and say so in the report.
