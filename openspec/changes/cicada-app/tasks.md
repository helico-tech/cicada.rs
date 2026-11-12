# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `json-schema-at-the-http-door` has landed when this is built (0.12.0).

## 1. The crate, with the command line in it

- [x] 1.1 The crate `cicada-app` [3, 5, 10]: a new member of the workspace with the modules of `cicada-cli` (the
      command line, its config, its output, and the terminal UI behind the feature `tui`) and the binary `cicada`.
      The features `sqlite`, `postgres` and `tui`; none is on by default. The crate `cicada-cli` leaves the
      workspace, and its tests move. The sample service names `cicada-app` with all three features from this task
      on, so that a build of the workspace has them on. Verify: `cargo test --workspace` runs the tests of the
      command line and of the terminal UI in the new crate; `cargo check -p cicada-app` passes with no feature and
      with each alone; and `grep -rn "cicada-cli\|cicada_cli" --include='*.toml' --include='*.rs' .` finds nothing
      outside `target`.
- [x] 1.2 The name, the version and the config file [1, 11]: the help, the usage errors and the hints name the
      binary as it was called; `--version` prints the host's text, or `cicada-app` and its version; the plain binary
      prints `cicada` and its version; a `cicada config` command writes through a symlink. Verify: tests for the
      scenarios "The name in a hint", "The version", "The version of the plain binary", and of `cli` "A path that
      is a symlink", "A relative link, and one whose target is not there", "Built with the feature" and "Built
      without the feature" (the last runs with `cargo test -p cicada-app`).

## 2. `App`

- [x] 2.1 The settings [2, 3, 4]: the flags and variables of the store, the address, the types and the node; the
      order of a flag, its variable, the host's `NodeConfig` and the default; durations; which store a value names;
      a value that is not a SQLite file is never printed. Verify: unit tests for the scenarios "A SQLite file",
      "PostgreSQL from a variable", "A flag wins", "No store", "An address of another kind", "A PostgreSQL address
      without its slashes", "A SQLite address", "A backend that is not built in" (a function over the list of
      backends that are built in), "The retention", "The pool", "The default address", "A variable over the host's
      value", "The host's value over the default", "The limit of lost claims is turned off", "A value that cannot
      be read", "A list", "None", "A type the binary does not have", "An empty name and a name twice", "A type
      called `all`" and "A worker that hosts nothing".
- [x] 2.2 `serve`, `worker` and the entry points [1, 2, 6, 7, 8, 9]: `App` with `host`, `observe`, `node_config`,
      `http`, `on_start`, `version`, `run`, `run_async` and `run_with`; `AppError`; `node_command`; the order of the
      start; `/ready` outside the host's router; the stop, with signals watched only for `serve` and `worker`; the
      wait for the store as a loop over a function that opens it; the refusal of the operator flags; no `serve` and
      no `worker` in a binary without types. Verify, with `run_with` on a SQLite file in a temporary folder and
      port 0: tests for the scenarios "Serve, create and send", "Stopping", "A request that stays open" (with a
      short wait), "The address it listens on", "The address is taken", "The node cannot start", "A stop while the
      store is waited for", "A stop during an operator command", "A stop while `on_start` runs", "A worker next to
      a serving node", "A layer of the host", "A route of the host", "A route `/ready` of the host", "A route of
      the host that clashes with the API", "An actor that must exist", "In a worker", "The function fails", "An
      observer", "A command of the host's binary", "No server", "An operator flag on `serve`", "An operator command
      of the plain binary", "No serve without types", "Help", "A command line that cannot be read", "Whether a node
      runs" and "A host with an async main"; on virtual time, with a function that fails, "The database comes up
      late" and "The database stays away"; and with `CICADA_PG_URL` set, one test that `worker` handles a message
      on PostgreSQL. Built: "A request that stays open" is a unit test of the inner function with a wait of 300 ms;
      "A host with an async main" needs a process and a signal, so it is a test of the sample service (3.1), whose
      binary is such a host: it serves, and SIGTERM ends it with exit code 0.

## 3. The sample service

- [x] 3.1 The sample on `App` [12]: its `main` on `run_async`, its types picked by `--types`, the node id it reads
      itself and checks in `on_start`, its routes and token through `http`, its singleton actors and its watcher in
      `on_start`, telemetry only for a node, `GET /` without the store, and the refusal of its old variables.
      Verify: the tests of the sample service pass, with a test that an old variable stops the start and that the
      error names the new one.
- [x] 3.2 The image and the end-to-end suite [12]: the Dockerfile (the entry point runs `serve`, the variables, the
      health check on the port of `CICADA_LISTEN`), the compose files, the code that starts single nodes, and the
      tests that read `GET /`. Verify: `cargo test -p cicada-e2e --no-run` passes; the suite runs in 6.2.

## 4. CI and the map

- [ ] 4.1 CI and `AGENTS.md` [10]: `cargo check -p cicada-app` with no feature and with each of `sqlite`, `postgres`
      and `tui` alone; `cargo test -p cicada-app`; a `cargo tree` check that `cicada-app` without `tui` names
      neither `ratatui` nor `crossterm`; the crate in the map and in the build lines of `AGENTS.md`. Verify: every
      command the CI file gained passes when it is run by hand, and the report lists them.

## 5. Docs

- [ ] 5.1 `README.md`: a quick start on `App`, with `--listen` and `App::http` where it first serves; the commands
      and the tables of flags and variables; how to install `cicada`; the variables of the sample service.
      `docs/operations.md`: deploying with `serve` and `worker`, that a worker has no readiness check, how a node
      stops, the loopback default, the pool and the retention, and that the API has no authentication unless the
      host adds a layer. `docs/architecture.md`: the crate table. The row of ADR 0027. Verify:
      `cargo test -p cicada-examples --doc` passes, and every risk of `design.md` that names the docs is in them.
- [ ] 5.2 `CHANGELOG.md`, a section `0.13.0 — unreleased`, and the version `0.13.0` in `Cargo.toml`: the steps of
      the Migration Plan of `design.md` first, then the breaking change of the proposal. Verify: every step of the
      plan and every **BREAKING** line of the proposal is in the section.

## 6. Checks

- [ ] 6.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, `cargo test -p cicada-app`, `cargo check -p cicada` and `cargo check -p cicada-app`
      each with no feature and with each feature alone, and `openspec validate cicada-app --strict`. Verify: all
      pass.
- [ ] 6.2 The PostgreSQL tests with `CICADA_PG_URL` set, once more against a throwaway `postgres:17.5-alpine` with
      its default 100 connections, as the database of CI has (removed afterwards), and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. All need Docker. Verify: all pass; if Docker cannot be
      started, leave this task open and say so in the report.
