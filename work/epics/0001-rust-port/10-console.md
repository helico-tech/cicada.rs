---
status: in-progress
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

- 2026-10-07: copied the web app to `crates/cicada-ui/web` (Vite output now configurable via `CICADA_UI_OUT_DIR`); `build.rs` runs `npm ci`/`npm run build` into `$OUT_DIR/web`, embedded with rust-embed (`debug-embed`). `actor_ui(mount_path, api_path)` returns absolute routes to merge, because axum 0.8's `nest` does not route `/console/` to a nested `/`. ActorUiTest ported (7 tests). Console unit tests (40) and typecheck pass.
- Open: the fixtures test (record every endpoint's responses from the Rust API and parse them with `generated.ts`), after story 08.

## Follow-ups

- (none yet)
