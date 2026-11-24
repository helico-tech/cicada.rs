# Proposal

## Why

Factory (cicada-code-factory) wants to move its container runs onto `cicada-containers` 0.20.7 and drop its own
`crates/containers` with its `docker` CLI backend (request from the Factory session, 2026-11-24; reference: Factory's
`docs/design/containers.md` and `crates/containers/src/runtime.rs`). It would be the third consumer (ADR 0056). Factory promises logs people can follow while a run goes on, stored outside the container. `logs(name,
tail_lines)` gives only the last lines as one string, at the end.

## What Changes

- `ContainerRuntime::logs_since(name, cursor, max_lines)` returns a page of lines `{time, stream: out | err, text}` and
  a cursor to call again from, on a running or ended container. The cursor is opaque: Docker's `since` has second
  granularity and several lines share a time, so a plain timestamp would repeat or drop lines.
- The Docker backend reads it with timestamps; the fake scripts lines with times.
- Text is lossy UTF-8, as `logs` is. It is called outside actor turns (logs are too big for actor state).

## Out of scope

A follower that stores logs (it stays with the host). Streaming over an open connection. The actor.

## Order

After `container-consistency`, before `container-copy-out`.

## Impact

The trait (an added method with a default that fails with `Other`, so no implementation outside this repo breaks; design
decision 5), the fake, the Docker backend and its E2E class. Additive for callers, a patch. Docs handoff: the runtime
section of the guide.
