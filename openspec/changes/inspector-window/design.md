# Design

Operators want to look inside a running system from a Mac with a native window, opened from the tray or on its own
(owner, 2026-11-26; mockup page Inspector). This first change gives the standalone window, its sidebar, the toolbar and
the Overview; the other sidebar items say "Coming soon" until their changes land. Opening it from the tray is the next
change, `inspector-in-tray` (design review: one change for both did not fit five tasks).

```rust
fn main() -> std::process::ExitCode {
    cicada_inspector::Inspector::new().run()   // standalone, on the current context of `cicada config`
}
```

## Decisions

1. **ADR 0076 supersedes ADR 0011 for a native macOS console only.** The repository holds a native macOS inspector,
   built in Rust on the HTTP API through `cicada-client`, in its own crate. ADR 0011's removal of the browser console
   stays, and so does ADR 0056's container exception. No library crate depends on `cicada-inspector`, with one allowed
   edge that `inspector-in-tray` adds: `cicada-tray`'s optional feature `inspector`. The gate's dependency check gains
   the crate with that exception named. ADR 0011's status line says it is partly superseded by 0076.
2. **A portable model, a thin AppKit view, as in the tray.** Crate `cicada-inspector` on `cicada-client` and
   `cicada-app`. Module `model` (every platform) gathers an `Overview`: `/stats`, `/health`, `/nodes`, the first page of
   `/turns` (20, with a "more" mark when the answer has a next cursor) and `/types`, then the actor count of each type
   from `/stats?prefix=<type>/`, at most 8 calls at a time and at most 50 types per refresh. The whole refresh has a
   deadline of 10 seconds. Each part has its own result: a value, an error (shown on its tile), or "not counted" for a
   type past the 50 or the deadline, which shows differently from a failed call. `failed` from `/stats` is `None` from
   an older server and shows as "—". A `Navigation` keeps the sidebar selection with back and forward. Module `appkit`
   (`cfg(target_os = "macos")`) renders and holds every `unsafe` call with its reason.
3. **The client from a context.** `cicada-app` gains `cicada_app::context_client(&Env, context: Option<&str>) ->
   anyhow::Result<(ActorClient, Option<String>)>`, built on the existing `config::resolve` and `Target::client` with
   the operator commands' timeout, through the sanitized loader of `tray-settings`: the same order as `cicada`
   (`CICADA_SERVER`, `CICADA_TOKEN`, the named context, `CICADA_CONTEXT`, the file's current context) and the effective
   context's name, so the picker shows what is really used. Errors shown in the window hide credentials in URLs
   (`scheme://user:***@host`, as the usage errors do).
4. **The window.** One `NSWindow` with an `NSSplitViewController`: a source-list sidebar with vibrancy (Overview;
   Browse: Actors, Messages, Running Turns, Outcomes, Nodes, Types, Watches), the content, and an `NSToolbar` with back
   and forward, the title, a context pop-up (names from `Contexts`, the effective one marked; picking one rebuilds the
   client and refreshes) and Refresh (⌘R). The Overview shows tiles (actors, pending, oldest due, failed labelled "kept
   by retention", as the API does not report the retention), the health, the nodes, the running turns and the types
   with their actor counts. Refresh is by hand; live updates come with `inspector-live`. macOS 14 or later, checked at
   run time as in the tray. A refresh started for one context and finished after a switch is discarded.
5. **Standalone threads.** `Inspector::run(self) -> ExitCode` owns the main thread, its `NSApplication` (regular
   policy, with a Dock icon and a main menu with Quit, ⌘W and ⌘R) and its own multi-threaded runtime. Calls publish
   snapshots through a `watch`; a main-thread timer reads it, as in the tray. Closing the window ends the app; the
   timer is invalidated and pending refreshes are dropped before the runtime shuts down. Elsewhere `run` returns an
   error that the inspector needs macOS 14. Embedding in another app's run loop is `inspector-in-tray`'s.

## Tests

Model, against an in-process server (`cicada-axum` on a `TestActors` store): the Overview's numbers, health, nodes, the
first page of running turns with its "more" mark, and the types with actor counts; one failing route marks only its
part; a type past the 50, or past the deadline, is "not counted", not failed; `failed` from an older answer is `None`;
at most 8 per-type calls at a time; a refresh finished after a context switch is discarded. `Navigation`: select, back,
forward, and a new selection after back drops the forward list. `context_client`: environment over context, the named
context, `CICADA_CONTEXT`, the file's current context and the effective name returned; no server fails with the operator
commands' text and without a token. The window gets a manual check list in the tasks.

## Docs

The handoff gives the app guide a short "Inspector" section (standalone), the docs index a line, and names the new
crate everywhere the crates are listed (architecture, AGENTS.md map, README crate table, install, consumer skill); the
changelog's 0.26.0 section names the crate and `context_client`.
