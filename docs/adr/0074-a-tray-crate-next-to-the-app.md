# 0074. A tray crate next to the app

Date: 2026-11-26

## Status

Accepted for tray-worker-control. Builds on ADR 0027 (the app layer) and ADR 0042 (app workers over HTTP).

## Context

Library users want a default way to run their actors from a Mac menu bar, the way `App` is the default binary. The menu
needs a model it can show and drive: connect, disconnect, the state of the connection, the registered types and the
turns that run.

## Decision

- A new crate `cicada-tray`, on `cicada-app`, which no other library crate depends on. Its model (`Tray`) is portable
  and tested everywhere; the macOS menu comes in later changes behind `cfg(target_os = "macos")`.
- A tray runs a worker over HTTP in its process, built fresh from a host's function for each connect, through
  `App::run_with` and a stop signal; it watches the state and records turns with a `TurnObserver`.
- `App::type_names()` lists the hosted types.

## Consequences

A host gets a menu bar worker by writing one function. Workers on the store and background tasks are not run by the
tray (an `App` with `on_start` or a task cannot run over HTTP). Windows and Linux are out of scope.
