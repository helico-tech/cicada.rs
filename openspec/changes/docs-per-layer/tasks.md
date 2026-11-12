# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `cicada-actors` has landed when this is built (0.14.0).

## 1. The guides and the README

- [x] 1.1 The four guides [1, 2]: `docs/guide/core.md`, `http.md`, `app.md` and `actors.md`, with the text of the
      README moved into them by the table of decision 3, each a doctest of `cicada-examples` (`examples/build.rs`
      expands them as it does the README; one doctest struct per guide in `examples/src/lib.rs`). A block that shows
      a worked example names its file of `examples/`. Verify: `cargo test -p cicada-examples --doc` passes and runs
      the blocks of the four guides (the report says how many per guide); a block changed so that it does not
      compile fails it.
- [x] 1.2 The README [3]: an overview of about 200 lines with the parts of decision 3; the table of the ADRs moves to
      `docs/adr/README.md`, the design notes and the known limitations to `docs/architecture.md`. The release steps
      still find the tag lines. Verify: `scripts/release-check.sh` still reads what it needs (run its check of the
      tag form on the current version); the doctests pass; the README names every guide.

## 2. The references

- [ ] 2.1 The help of the command line [5]: a unit test inside `cicada-app`, behind the feature `tui`, that walks the
      commands with `get_subcommands()`, renders the help of `cicada` and of `serve` and `worker` (from an `App` with
      one test type named `myapp`), and compares it with `docs/reference/cli.md`, rewriting it with
      `CICADA_UPDATE_CLI=true`; its failure message names `--workspace` or `--features tui`. Verify: tests for the
      scenarios "A flag is added", "A subcommand is added" (each tried once in a scratch change and put back) and
      "The name of the test binary"; the reference is committed.
- [ ] 2.2 The contracts of the ready-made actors [6]: the snapshots of `cicada-actors` move to
      `docs/reference/contracts/`, and its test compares each as pretty JSON text, descriptions included. Verify:
      tests for the scenarios "A field of a ready-made actor changes" and "Only a doc comment changes" (each tried
      once and put back).

## 3. The checks of the docs

- [ ] 3.1 The table of guarantees [7]: `docs/guarantees.md` with a row for each of G1–G6 and S1–S10 and the tests
      that prove each, and a test of `cicada-examples` that checks it by the rules of decision 7. Verify: tests for
      the scenarios "A named test is gone", "A guarantee without a row" and "A table that cannot be read"; the table
      names at least one test for every row, and the report lists them.
- [ ] 3.2 The links [8]: a test of `cicada-examples` that checks the relative links and anchors of the Markdown files
      of the repository outside `target` and `openspec/changes/archive`; every pointer into the README that moved is
      fixed (`docs/architecture.md`, `docs/operations.md`, `AGENTS.md`, `openspec/config.yaml`, the proposal of
      `value-depth-limit`). Verify: tests for the scenarios "A section moves" and "A link to the web"; the test
      passes on the repository.
- [ ] 3.3 `missing_docs` [9]: `deny` in the workspace lints. Verify: the scenario "A public function without a doc
      comment" (tried once and put back); `cargo build --workspace` passes.
- [ ] 3.4 The Purposes of the specs [10]: the eleven specs with the placeholder get a real Purpose. Verify:
      `openspec validate --specs --strict` passes.

## 4. Docs of the change

- [ ] 4.1 `AGENTS.md` [11]: the map, the build lines and the rules name the guides, the references and their
      variables, the table of guarantees, the link check and the index of the ADRs. Verify: the link check passes, and
      `grep -n "user guide" AGENTS.md` names the guides, not the README.
- [ ] 4.2 `CHANGELOG.md`, a section `0.14.1 — unreleased`, and the version `0.14.1` in `Cargo.toml`: what moved where,
      the new references and checks, and that nothing changes for a host. Verify: the section names every guide and
      every new check.

## 5. Checks

- [ ] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, `cargo test -p cicada-examples --doc`, `cargo check -p cicada` with no feature and with
      each feature alone, `openspec validate docs-per-layer --strict` and `openspec validate --specs --strict`.
      Verify: all pass.
- [ ] 5.2 The PostgreSQL tests with `CICADA_PG_URL` set, once more against a throwaway `postgres:17.5-alpine` with
      its default 100 connections (removed afterwards), and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. All need Docker. Verify: all pass; if Docker cannot be
      started, leave this task open and say so in the report.
