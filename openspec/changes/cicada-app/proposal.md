# Proposal

## Why

Getting one actor to run behind HTTP takes about 150 lines of host code (ADR 0018): open a store, build the system,
mount the routers, read the configuration, shut down cleanly. The sample service and every host repeat it. The
operator tool is a crate of its own, apart from the binary that serves. ADR 0018 adds a layer that turns a set of
actors into a runnable binary.

## What Changes

- A new crate `cicada-app` (layer 2). `App::new().host(actor).run()` gives a binary with three kinds of commands:
  `serve` (worker and HTTP), `worker` (types, no HTTP) and the operator commands.
- The store and the listen address come from flags or environment variables (`--store`, `CICADA_STORE`, `--listen`,
  `CICADA_LISTEN`), and so do the settings of the node. `serve` listens on `127.0.0.1:8080` unless told otherwise.
- `serve` and `worker` take `--types`, to host a part of the binary's types; `serve --types none` serves HTTP and
  hosts nothing.
- `App` only composes layers 0 and 1. A host adds its layer and its routes with `App::http`, work after the start
  with `App::on_start`, observers with `App::observe` and its `NodeConfig` with `App::node_config`. A host that
  needs more uses the layers directly.
- The operator commands always go over HTTP, also when they run in the serving binary.
- **BREAKING** The crate `cicada-cli` goes: its command line moves into `cicada-app`, which still builds the binary
  `cicada` with the operator commands. The terminal UI is a feature of `cicada-app` that is off by default.
- A binary without hosted types has no `serve`; `serve` and `worker` refuse the flags of the operator commands.
- `cicada config` commands write through a config path that is a symlink.
- The sample service is rebuilt on `App`. Its variables for the store, the port, the types and the node settings
  are replaced by those of `App`, and its `GET /` no longer names the store. For this release it refuses to start
  when an old variable is set.

The simplest version: flags and environment variables only, no config file for the server.

## Capabilities

### New Capabilities

- `app`: a set of actors becomes a binary that serves, works and takes operator commands.

### Modified Capabilities

- `cli`: the terminal UI sits behind a feature of the build, and a config path that is a symlink is written
  through.

## Out of scope

- TLS, and authentication schemes of its own.
- Packaging and publishing binaries.
- A config file for the server.
- OpenTelemetry in `cicada-app`: a host adds `cicada-otel` as an observer.

## Order

Step 9 of ADR 0018. After `json-schema-at-the-http-door`, before `cicada-actors`. It needs `private-store-trait` (a
store opened from a path or a URL), and `cli-and-tui` must be archived first, because that change creates the `cli`
spec this one modifies.

## Impact

- A new crate; `cicada-cli` (moved into it), the sample service and its tests, `e2e` (the compose files, the
  variables of the sample service, the code that starts single nodes, the tests that read `GET /`), the image of
  the sample service, the workspace manifest and CI (the crate list, `cicada-app` with no feature and with each
  alone),
  `README.md`, `AGENTS.md` (the map), `docs/operations.md`, `docs/architecture.md` (the crate table) and
  `CHANGELOG.md`.
- No change to a guarantee or the wire format. ADR 0018 covers the layering; ADR 0027 records where this change
  departs from it (`--types` on `serve` too, no `serve` in a binary without types) and the names of the flags and
  variables, which hosts will depend on.
