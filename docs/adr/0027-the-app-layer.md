# 0027. The app layer

Date: 2026-11-12

## Status

Accepted

## Context

ADR 0018 adds `cicada-app`, a layer that turns a set of actors into a binary with the commands `serve`, `worker`
and the operator commands, configured by flags and environment variables. It leaves the details to the change
`cicada-app`. That change departs from ADR 0018 in two places, and it fixes names that hosts and their deployments
will depend on.

## Decision

- **`serve` takes `--types` too.** ADR 0018 gives a subset of the types to `worker` only. A node that serves HTTP
  and hosts a part of the types, or none, is what a system with more than one kind of node runs.
- **A binary without hosted types has neither `serve` nor `worker`.** The plain `cicada` binary is built from `App`
  without types and has the operator commands only. It must not serve: nobody wrote a layer for it.
- **`App` has no authentication of its own, and `serve` listens on `127.0.0.1:8080` unless told otherwise.** A host
  adds its layer with `App::http` and opens the address with `--listen`. `serve` and `worker` refuse the flags of
  the operator commands, so that `serve --token` cannot look like protection.
- **The node stops before the HTTP server.** The API keeps answering while turns finish, and `/ready` answers `503`
  from the start of the stop.
- **Every setting has a flag and a variable that starts with `CICADA_`:** the store (`--store`, `--store-schema`,
  `--store-pool-size`, `--retention`), the address (`--listen`), the types (`--types`) and the fields of
  `NodeConfig` that an operator changes per deployment. A flag wins over its variable, the variable over the
  `NodeConfig` of the host, and that over the default of the library.
- **The crate `cicada-cli` goes.** Its command line is part of `cicada-app`; the terminal UI is a feature that is
  off by default, and so is each store: a host names its store, as it does with the core.

## Consequences

- A binary that only serves, without actor code, cannot be built on `App`. A node that only sends is a host's
  binary run with `serve --types none`.
- A host that runs `serve` in a container sets `--listen`, or nothing answers from outside.
- A `worker` has no readiness check.
- The names of the flags and variables are part of what a release may not change lightly.
