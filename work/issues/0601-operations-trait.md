---
status: todo
type: idea
created: 2026-10-07
---

# 0601. An operations trait shared by ActorSystem and the HTTP client

## Problem

Kotlin's `FailureWatcher` takes `ActorOperations`, an interface implemented by both the node (`ActorSystem`) and the
HTTP client (`ActorClient`), so the watcher can run in a separate process. The Rust `FailureWatcher` takes an
`ActorSystem`, because the core has no such trait.

## Notes

- A small async trait with `create`, `send_with` and `failures`, implemented by `ActorSystem` and by
  `cicada-client`, would restore the Kotlin flexibility. Decide when story 09 (client) lands.
