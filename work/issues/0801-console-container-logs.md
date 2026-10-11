---
status: todo
type: idea
created: 2026-10-11
---

# 0801. Container logs in the console

## Problem

The console shows a container handle's state, but not its logs; those are only at `GET /containers/{name}/logs`.

## Notes

- On the actor page of an `actor://containers/{name}` instance, a Logs widget that reads the logs router (tail, follow).
- The console only knows the API path; the logs router must be mounted at the same base, or its path configured.
- Needs a console change in `crates/cicada-ui/web` and the fixtures test.
