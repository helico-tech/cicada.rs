# Design

The tray's menu should open the inspector in the tray's own process, on the tray's context. `inspector-window` gave a
standalone inspector that owns its app, run loop and runtime; this change lets it live inside an app that already runs.

## Decisions

1. **An embedded controller in `cicada-inspector`, sharing the window's teardown.** `Embedded::new(mtm, env, handle)`
   (macOS only) keeps the host's main-thread marker, environment and Tokio handle; `show(context: Option<&str>)` builds
   the window on that context when none is open, else brings the open one to the front without changing its context;
   `close()` closes it. One teardown, shared by the embedded and the standalone window, runs on every close: it
   invalidates the window's timer, bumps the window's generation so a refresh result that arrives later is discarded,
   and aborts the pending refreshes, which run on the given handle. Close does not wait for an aborted task to finish
   (`abort` only requests it): a late result is dropped by the generation check, and the host's runtime shutdown
   bounds the rest. A later `show` builds a new window with a new generation. Dropping the controller closes. It never
   touches the app's activation policy, delegate, main menu or run loop. The standalone `Inspector::run` uses the same
   window and teardown and keeps its own exit when its window closes.
2. **Focus and keys, as the Settings window.** `show` activates the app and makes the window key and front, as
   Settings… does. Refresh (⌘R) is the key equivalent of the toolbar's Refresh button and ⌘W closes, both handled by
   the window itself (its `performKeyEquivalent`), so each reaches the key window when the inspector and Settings are
   both open; back and forward keep ⌘[ and ⌘]. ⌘I is the key equivalent of the tray menu's item, so it works while the
   menu is open, not globally. The standalone app keeps its main menu as well.
3. **The tray's feature.** `cicada-tray` gains `[features] inspector = ["dep:cicada-inspector"]` (the one edge ADR 0076
   allows; the dependency check names it and is run with the feature on and off). With it, the menu model has "Open
   Inspector…" (⌘I) after Settings…; the AppKit view keeps one `Embedded` in its main-thread state, created on first
   use. The context passed is the connected one, else `None`. With `None` the inspector resolves its target as `cicada`
   does (`CICADA_SERVER`, `CICADA_CONTEXT`, the file's current context); its "no context" state shows only when that
   finds no server.
4. **Quit and reconnect.** Every quit path of the tray (the Quit item, Ctrl-C, and a system terminate answered now or
   later) closes the inspector first, with decision 1's teardown, and then goes on as before; while the tray is
   quitting, Open Inspector… is disabled and `show` does nothing. A reconnect of the tray, or a context pick in the
   menu, does not change an open inspector's context: the inspector has its own picker.
5. **The gate builds and tests the feature.** `scripts/ci/feature-builds.sh` adds `cargo check`, `cargo clippy
   --all-targets` and `cargo test` of `cicada-tray` with `--features inspector`, all with warnings denied, as the
   gate's workspace runs go without the feature. On a machine other than macOS they build and test only the portable
   parts.

## Tests

The embedded controller's lifecycle as a plain state machine: closed, open, show while open keeps the context, close,
show after close builds a new window with a new generation, a result of an old generation dropped, quit closes. The
tray: the menu model has the item only with the feature, and disabled while quitting; every quit path closes the
inspector first; the context passed (connected, else `None`). The dependency check with the feature on and off. The
window inside the tray gets a manual check list in the tasks: focus next to Settings, ⌘W and ⌘R with both windows open,
⌘I from the menu, quit with a refresh pending.

## Docs

The handoff gives the app guide's tray section "Open Inspector…" and the feature, and the changelog's 0.26.0 section
names the feature and `Embedded`.
