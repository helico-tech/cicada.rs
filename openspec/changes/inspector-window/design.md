# Design

Operators want to look inside a running system from a Mac with a native window, opened from the tray or on its own
(owner, 2026-11-26; mockup page Inspector). This first change gives the window, its sidebar, the toolbar and the
Overview; the other sidebar items say "Coming soon" until their changes land.

```rust
fn main() -> std::process::ExitCode {
    cicada_inspector::Inspector::new().run()   // standalone, on the current context of `cicada config`
}
```

## Decisions

1. **ADR 0076 supersedes ADR 0011's "no console".** The repository holds a native macOS inspector, built in Rust on the
   HTTP API through `cicada-client`, in its own crate that no library crate depends on (the gate's dependency check
   gains it, as for `cicada-tray`). Still no browser console and no container handles. ADR 0011's status line says it is
   partly superseded by 0076.
2. **A portable model, a thin AppKit view, as in the tray.** Crate `cicada-inspector` on `cicada-client` and
   `cicada-app`. Module `model` (every platform): `Overview` gathered from `/stats`, `/health`, `/nodes`, `/turns` and
   `/types`, plus one `/stats?prefix=<type>/` per type for its actor count (at most 8 at a time); each part has its own
   result, so a failed call shows on its tile and the others still show. `failed` from `/stats` is `None` from an older
   server and shows as "—". A `Navigation` keeps the sidebar selection with back and forward. Module `appkit`
   (`cfg(target_os = "macos")`) renders and holds every `unsafe` call with its reason.
3. **The client from a context.** `cicada-app` gains `cicada_app::context_client(&Env, context: Option<&str>) ->
   anyhow::Result<ActorClient>`: the same resolution as the operator commands (`CICADA_SERVER`, `CICADA_TOKEN`, the
   named or current context, its headers) with their timeout, so the inspector sees what `cicada` sees. Errors carry no
   token (the shared loader's sanitizing, from `tray-settings`).
4. **The window.** One `NSWindow` with an `NSSplitViewController`: a source-list sidebar with vibrancy (Overview;
   Browse: Actors, Messages, Running Turns, Outcomes, Nodes, Types, Watches), the content, and an `NSToolbar` with back
   and forward, the title, a context pop-up (names and current from `Contexts`; picking one rebuilds the client and
   refreshes) and Refresh (⌘R). The Overview shows tiles (actors, pending, oldest due, failed, labelled "kept by
   retention", as the API does not report the retention), the health, the nodes, the running turns and the types with
   their actor counts. Refresh is by hand; live updates come with `inspector-live`. macOS 14 or later, checked at run
   time as in the tray.
5. **Threads.** Calls run on a Tokio runtime and publish snapshots through a `watch`; a main-thread timer reads it, as
   in the tray. `Inspector::run(self) -> ExitCode` owns the main thread and its own runtime; elsewhere it returns an
   error that the inspector needs macOS 14.
6. **From the tray.** `cicada-tray` gains the feature `inspector` (an optional dependency on `cicada-inspector`). With
   it the menu has "Open Inspector…" (⌘I), which opens the window in the tray's process, on the tray's runtime, on the
   connected context or else the current one; a second pick brings it to the front. `cicada-inspector` exposes
   `Inspector::open(mtm, context)` for that, which needs an `NSApplication` already running.

## Tests

Model, against an in-process server (`cicada-axum` on a `TestActors` store): the Overview's numbers, health, nodes,
running turns and types with actor counts; one failing route marks only its part; `failed` from an older answer is
`None`; the per-type calls are limited to 8 at a time. `Navigation`: select, back, forward, and a new selection after
back drops the forward list. `context_client`: environment over context, the named context, no context and no
server fail with the operator commands' text and without a token. The window and the tray's item get a manual check
list in the tasks.

## Docs

The handoff gives the app guide a short "Inspector" section (standalone and from the tray), the docs index a line, and
names the new crate everywhere the crates are listed (architecture, AGENTS.md map, README crate table, install,
consumer skill); the changelog's 0.26.0 section names the crate, the tray feature and `context_client`.
