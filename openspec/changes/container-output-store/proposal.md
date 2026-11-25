# Proposal

## Why

Factory keeps each run's logs and files in its own store (a folder per run) and reads them back for an HTTP route.
Any host that runs containers needs the same, so `cicada-containers` gets the interface and the later changes of this
line write through it (Factory's `store.rs`, about 1,000 lines with the runner, request from the Factory session, 2026-11-25; the owner accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- A trait the host implements, such as `OutputStore`, per container name: append log lines together with the log
  cursor they end at (one write, so a crash neither repeats nor loses lines), read the last cursor, read lines back
  from an offset with a limit, put a file, list and read stored files, and delete everything of a container.
- An in-memory implementation for tests (feature `testing`), and a folder implementation (one folder per container)
  if the design finds it small enough; Factory keeps its own.

To decide in the design: sync or async, the error type, the offset (line number or byte), how a file is named, whether
the folder implementation belongs here.

## Out of scope

Who writes to the store (the next changes). HTTP routes.

## Order

After `app-tasks`, before `container-log-archive`.

## Impact

`crates/cicada-containers` (a new module). Additive.
