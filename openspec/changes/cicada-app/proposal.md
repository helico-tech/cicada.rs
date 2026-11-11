# Proposal

## Why

Getting one actor to run behind HTTP takes about 150 lines of host code (ADR 0018): open a store, build the system,
mount the routers, read the configuration, shut down cleanly. The sample service and every host repeat it. The
operator tool is a crate of its own, apart from the binary that serves. ADR 0018 adds a layer that turns a set of
actors into a runnable binary.

## What Changes

- A new crate `cicada-app` (layer 2). `App::new().host(actor).run()` gives a binary with three kinds of commands:
  `serve` (worker and HTTP), `worker` (a subset of the types, no HTTP) and the operator commands.
- The store and the listen address come from flags or environment variables.
- `App` only composes layers 0 and 1. A host that needs more uses those layers directly.
- The operator commands always go over HTTP, also when they run in the serving binary.
- `cicada-cli` becomes the command line of this crate. The terminal UI is a feature that is off by default.
- The sample service is rebuilt on `App`.

The simplest version: flags and environment variables only, no config file for the server.

To decide in the design:

- How a host adds its authentication to `serve`. Today the host wraps the routers itself.
- The names of the flags and environment variables.
- Whether the standalone `cicada` binary stays, or operators use the host's binary.
- A config path that is a symlink. The `cicada config` commands save by renaming a new file over the path, so they
  replace the link with a regular file and the link's target keeps the old text. Resolve the link before saving, or
  say so in the README (found while `cli-and-tui` was validated).

## Capabilities

### New Capabilities

- `app`: a set of actors becomes a binary that serves, works and takes operator commands.

### Modified Capabilities

- `cli`: the command line moves into `cicada-app`, and the terminal UI sits behind a feature.

## Out of scope

- TLS, and authentication schemes of its own.
- Packaging and publishing binaries.
- A config file for the server.

## Order

Step 9 of ADR 0018. After `json-schema-at-the-http-door`, before `cicada-actors`. It needs `private-store-trait` (a
store opened from a path or a URL), and `cli-and-tui` must be archived first, because that change creates the `cli`
spec this one modifies.

## Impact

- A new crate; `cicada-cli` (moved into it), the sample service, `e2e`, `README.md`, `docs/operations.md`,
  `docs/architecture.md` (the crate table) and `CHANGELOG.md`.
- No change to a guarantee or the wire format; ADR 0018 covers the layering.
