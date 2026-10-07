---
status: todo
type: debt
created: 2026-10-07
---

# 0602. `croner` is unused

## Problem

The root `Cargo.toml` lists `croner` in `[workspace.dependencies]` for the extras' "cron schedule". The Kotlin
`Schedule` is not cron: it has its own format (`every` N seconds, `daily` at hh:mm UTC), which `cicada-extras` matches,
so no crate uses `croner` any more.

## Notes

- Remove it from `[workspace.dependencies]`, or add a cron variant to `Schedule` if one is wanted (that would be a
  new feature for both implementations, so the wire form should be agreed first).
