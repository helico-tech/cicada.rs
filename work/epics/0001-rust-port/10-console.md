---
status: todo
created: 2026-10-07
---

# 10. Embed the React console and serve it with `actor_ui()`

## Goal

Embed the React console and serve it with `actor_ui()`.

## Acceptance criteria

- Console builds and is embedded; CSP and config.json as Kotlin.
- Fixtures of every endpoint parse with the console's zod types.

## Plan

Copy `actors-ui/src/main/web`, add rust-embed serving, port `ActorUiTest` and the fixtures test.

## Progress log

- (none yet)

## Follow-ups

- (none yet)
