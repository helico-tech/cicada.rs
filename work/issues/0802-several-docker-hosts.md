---
status: todo
type: idea
created: 2026-10-11
---

# 0802. Container handles on several Docker hosts

## Problem

Every node that hosts `ContainerActor` must reach the same Docker daemon (ADR 0007). Spreading containers over several hosts is not possible.

## Notes

- Put the host in the handle (`actor://hosts/{host}/containers/{name}`) and keep a map of host to engine on the actor.
- Every hosting node must still reach every daemon (TCP/TLS), because any of them may run the next turn.
- Only worth it when a real use needs it.
