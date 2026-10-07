---
status: done
type: debt
created: 2026-10-07
---

# 0700. `TestActors` has no `list(prefix)`

## Problem

Kotlin's `testActors { }` scope has `list(prefix)` (the `GET /actors?prefix=` listing). Three example tests use it
(`BatchTest`, `RegistryTest`, `CleanupTest`). `cicada_test::TestActors` has no such method, so the Rust example tests
call `test.system().list_actors(prefix, None, 100)` through a helper in `examples/tests/common/mod.rs`.

## How to reproduce

Port a Kotlin test that calls `list("actor://devices/")`: there is no `TestActors` method for it.

## Notes

- Add `TestActors::list(&self, prefix: &str) -> Vec<ActorAddress>` that pages through `Store::list`, like `pending`
  and `failures` already do. Then drop the helper in `examples/tests/common/mod.rs`.
