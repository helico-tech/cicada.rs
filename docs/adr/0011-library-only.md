# 0011. A library only: no console, no container handles

Date: 2026-11-09

## Status

Accepted. Supersedes 0005, 0006 and 0007. Partly superseded by [ADR 0056](0056-shared-external-resource-crates.md):
no container handles.

## Context

Cicada for Rust is meant to be a durable actor library that is complete, opinionated and not bloated. Two crates in the
workspace were not part of that:

- `cicada-ui`, the actor console (ADR 0005, ADR 0006): a React app built with npm and Vite in a Cargo build script.
  Every build of a crate that depended on it needed Node.js 22, npm and the network. It carried a vendored design
  system and its own wire types, kept in step with the Rust API by hand.
- `cicada-docker`, container handles (ADR 0007): actors that run and watch Docker containers. It used only the public
  API, so it is an application of the library, not a part of it. It brought bollard into the workspace, and three of
  the four open changes were for it.

## Decision

Both crates are removed from this repository. The workspace holds the library (core, stores, HTTP API, client, test
harness, plugins and building blocks), its examples, the sample service and the end-to-end tests.

- There is no console. The HTTP API (`cicada-axum`) and the client (`cicada-client`) are how tools talk to a node. A
  console can be built on them in a repository of its own.
- There are no container handles. An actor type that manages something outside Cicada lives with the application that
  needs it. The last version of both crates is in git history (commit `7d43cf0`).
- `sample-service` no longer serves `/console/`, has no `CONTAINERS` setting and no `/containers/{name}/logs`.

## Consequences

- Building the workspace needs only Rust. CI and the sample image no longer install Node.js.
- `rust-embed`, `mime_guess` and `bollard` leave the dependency tree.
- The HTTP API has no in-repository browser client any more. Endpoints that only the console called
  (`GET /messages?prefix=`, `GET /messages/{id}/chain`, `GET /types?uri=`) have no caller left besides the client and
  the tests; whether they stay is decided separately.
- The open changes `docker-events-watcher`, `console-container-logs` and `several-docker-hosts` have nothing to change
  here any more.
- Anyone who used `cicada-ui` or `cicada-docker` as a git dependency must pin a revision from before this change.
