# Proposal

## Why

The HTTP API is the only way to look at a running system from outside, and today that means `curl` and `jq`. An
operator who wants to see what is stuck, cancel a message or pause an actor has to know the paths and query parameters
by heart. Kubernetes users have `kubectl` for one-off commands and `k9s` for watching. Cicada needs the same two things.

## What Changes

- A new crate `crates/cicada-cli` with one binary, `cicada`. It talks to a node only over HTTP, through
  `cicada-client`, so it works against any node that serves `actor_api()` (or `actor_api_read()` for the reading
  commands).
- kubectl-style commands, verb first: `get`, `describe`, `create`, `delete`, `send`, `cancel`, `retry`, `deliver-now`,
  `pause`, `resume`, `purge`, `stats`, `health`, and `config`. Output as a table (default), `-o wide`, `-o json` or
  `-o yaml`.
- Named contexts in a TOML file (`~/.config/cicada/config.toml`), each with a server URL, an optional bearer token and
  optional headers. The flags `--server`, `--token` and `--context` win over the environment variables `CICADA_SERVER`,
  `CICADA_TOKEN` and `CICADA_CONTEXT`, and those win over the file.
- `cicada tui`: a k9s-style terminal UI. It lists actors, pending messages, outcomes, turns, nodes and types, refreshes
  them on its own, opens one item as JSON, and runs single-item actions (delete, cancel, retry, pause/resume, purge,
  deliver-now) after a confirmation.

The simplest version: no code generation. `cicada-client` already covers every operation of the OpenAPI document and
shares the wire types with the server; the README says why it stays hand-written (the document is OpenAPI 3.1).

## Capabilities

### New Capabilities

- `cli`: the `cicada` command line and terminal UI: how it finds its server, what commands it has, and how it shows
  results.

### Modified Capabilities

None.

## Out of scope

- Generating a client from `docs/openapi.json`.
- Bulk actions and sending messages from the TUI (the CLI has both).
- Watching with `-w` on the CLI; the TUI is the way to watch.
- Authentication other than a bearer token or fixed headers (no OAuth flows, no client certificates).
- Installing or publishing the binary (packages, Homebrew). The workspace stays `publish = false`.
- Shell completion scripts.

## Order

The first one: it is built and has one docs task left. Before `every-actor-has-state`. It is not a step of ADR 0018,
but step 9 (`cicada-app`) needs it archived.

## Impact

- New crate `crates/cicada-cli`, added to the workspace. New dependencies: `clap`, `ratatui`, `crossterm`, `toml`,
  `serde_norway`.
- README (a section on the CLI), `docs/architecture.md` (the crate table).
- No change to the library crates, the wire format or any guarantee, so no ADR.
