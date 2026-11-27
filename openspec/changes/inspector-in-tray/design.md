# Design

The tray's menu should open the inspector in the tray's own process, on the tray's context. `inspector-window` gave a
standalone inspector that owns its app, run loop and runtime; this change lets it live inside an app that already runs.

## Decisions

1. **An embedded controller in `cicada-inspector`.** `Embedded::new(mtm, env, handle)` (macOS only) keeps the host's
   main-thread marker, environment and Tokio handle; `show(context: Option<&str>)` builds the window on that context
   when none is open, else brings the open one to the front without changing its context; `close()` closes it. Closing,
   by the user or by `close()`, invalidates the window's timer and aborts its pending refreshes (their `JoinHandle`s on
   the host's runtime) before the window is released; a later `show` builds a new one. Dropping the controller closes.
   It never touches the app's activation policy, delegate, main menu or run loop. The standalone `Inspector::run` is
   built on the same window code, so both paths share it.
2. **Keys without a main menu.** An embedded window cannot rely on the app's main menu (the tray's is hidden, with
   only Edit). Refresh (⌘R) is the key equivalent of the toolbar's Refresh button, and ⌘W closes through the window's
   own key handling, as the Settings window does; back and forward keep ⌘[ and ⌘]. The standalone app keeps its main
   menu as well.
3. **The tray's feature.** `cicada-tray` gains `[features] inspector = ["dep:cicada-inspector"]` (the one edge ADR 0076
   allows; the dependency check already names it). With it, the menu model has "Open Inspector…" (⌘I) after Settings…;
   the AppKit view keeps one `Embedded` in its main-thread state, created on first use. The context is the connected
   one, else the current one of `Contexts`, else none (the inspector then shows its "no context" state). Because the
   tray is an accessory app, the view activates the app before showing the window, so it comes to the front.
4. **Quit and reconnect.** The tray's quit closes the inspector first (decision 1's close), then goes on as before, so
   no refresh outlives the runtime. A reconnect of the tray, or a context pick in the menu, does not change an open
   inspector's context: the inspector has its own picker.
5. **The gate builds the feature.** `scripts/ci/feature-builds.sh` adds `cargo check -p cicada-tray --features
   inspector` and `cargo clippy -p cicada-tray --features inspector --all-targets`, both with warnings denied, as the
   gate's workspace clippy runs without the feature. On a machine other than macOS they build only the portable parts.

## Tests

The embedded controller's lifecycle as a plain state machine (closed, open, show while open keeps the context, close,
show after close builds a new window, quit closes and drops refreshes). The tray: the menu model has the item only with
the feature; the context rule (connected, else current, else none). The script tests if the feature-builds script has
tests. The window inside the tray gets a manual check list in the tasks.

## Docs

The handoff gives the app guide's tray section "Open Inspector…" and the feature, and the changelog's 0.26.0 section
names the feature and `Embedded`.
