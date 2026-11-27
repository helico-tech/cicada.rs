# Proposal

## Why

The tray's menu should open the inspector in the tray's own process, on the tray's context (owner's proposal for
`inspector-window`). It was split off from `inspector-window`, whose design review found that embedding a second
window in the tray's `NSApplication` needs its own controller and lifecycle, and that both together did not fit five
tasks (2026-11-27).

## What Changes

- `cicada-tray` gains the feature `inspector`, an optional dependency on `cicada-inspector` (the one edge ADR 0076
  allows). With it, the menu has "Open Inspector…" (⌘I), which opens the window on the connected context, else the
  current one; a second pick brings it to the front.
- `cicada-inspector` gains a way to open in an app that already runs (for example `Inspector::open(mtm, env,
  runtime_handle, context)`): a main-thread controller, the host's environment and Tokio handle, and a close path that
  invalidates its timer and drops stale refreshes. The tray keeps its own delegate, accessory policy and run loop.

To decide in the design: the API, what happens to the window when the tray quits or reconnects, and focus between the
inspector and the Settings window.

## Order

After `inspector-window`, before `inspector-actors`.

## Impact

Crates: cicada-inspector, cicada-tray.

`crates/cicada-inspector` (embedding), `crates/cicada-tray` (feature and menu item). The gate builds the feature on
macOS; a manual check list for focus, shortcuts, close and reopen, and quit with the window open. Docs handoff: the
app guide's tray section.
