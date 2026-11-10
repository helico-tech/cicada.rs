# Tasks

## 1. Crate and configuration

- [x] 1.1 Add `crates/cicada-cli` (library + binary `cicada`) to the workspace with `clap`, `ratatui`, `crossterm`,
      `toml` and `serde_norway` as workspace dependencies. Verify: `cargo build -p cicada-cli` and `cicada --help`.
- [x] 1.2 `config.rs`: the file model, the path rules and target resolution as a pure function of flags, an environment
      map and the file; build a `reqwest::Client` with the token and headers. Verify: unit tests for flag over env over
      file, unknown context, no server, the path rules and a file round trip.

## 2. Commands and output

- [x] 2.1 `output.rs`: the `Columns` trait for every view type, the table printer, JSON and YAML output and the age
      format. Verify: unit tests of the age format and of a table built from fixed views.
- [x] 2.2 `cli.rs` and `run`: every command of the design table, `--all` paging, missing items and API errors as exit
      code 1. Verify: `tests/cli.rs` against a real `actor_api()` node covers each command, each output format, a
      missing item and an API error.
- [x] 2.3 `config` commands (`get-contexts`, `current-context`, `use-context`, `set-context`). Verify: tests on a
      temporary config file.

## 3. TUI

- [x] 3.1 `tui/app.rs` and `tui/grid.rs`: the screen stack, input, effects, filter, sort and selection, without I/O.
      Verify: unit tests for commands and completion, the live filter, the confirmation, action resolution,
      drill-down and back, late answers, the tick schedule and the mouse.
- [x] 3.2 `tui/fetch.rs`, `tui/theme.rs`, `tui/highlight.rs`, `tui/ui.rs` and `tui/mod.rs`: the overviews, colours,
      header, dialogs and the select loop; terminal and mouse restored on exit, error and panic. Verify: `TestBackend`
      renders, and a manual run against `sample-service` with token auth.

## 4. Docs and checks

- [ ] 4.1 README: a section on the CLI and TUI (config file, commands, keys); `docs/architecture.md`: the crate table.
      Verify: `cargo test -p cicada-examples --doc` passes.
- [x] 4.2 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
      `openspec validate cli-and-tui --strict`. Verify: all pass.
