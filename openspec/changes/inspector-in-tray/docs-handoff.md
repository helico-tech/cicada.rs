# Docs handoff for inspector-in-tray

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The optional `cicada-tray` feature `inspector` adds Open Inspector… after Settings…. It opens the inspector in the
tray's process. The initial context comes from the tray's connection, including while it is connecting or stopping.
Without a connection, the inspector resolves the target as `cicada` does using the environment and config file.
A server supplied by `CICADA_SERVER` can be used without a named context.

On macOS 14 or later, hosts can create a `cicada_inspector::Embedded` with `new(mtm, env, handle)`. Call
`show(Some("prod"))` to open on a context, or `show(None)` to use the same target resolution as `cicada`. Calling
`show` again brings the open window forward and preserves its current context. Call `close()` or drop the
controller to close the window and request cancellation of its refresh. The host owns the app and Tokio runtime.

The portable menu tests in `crates/cicada-tray/src/menu.rs` cover menu visibility and disablement, context routing
and the ordering of the inspector close on all quit paths. The inspector tests cover closing during a refresh,
reopening with a fresh session, the server-only target and a malformed config.

## 1. Open Inspector from the tray

**Where.** `docs/guide/app.md`, `Menu bar app on macOS`, after Settings.

**Text.** Enable the Cargo feature `inspector` on `cicada-tray`. Choose Open Inspector… (⌘I while the tray menu
is open) to open the inspector on the tray's connected context. When the tray is offline, the inspector starts
on the target `cicada` would use. The inspector has its own context picker. A second Open Inspector… brings the
same window forward on the context it is already showing. Closing it and opening it again picks the initial target
from the tray's latest connection state.

When the inspector is the key window, ⌘R refreshes, ⌘W closes and ⌘[ and ⌘] navigate the sidebar history. Quit,
Ctrl-C and logout close the inspector and request cancellation of its refresh before the tray finishes quitting.

**Caveats.** The tray and embedded inspector require macOS 14 or later. `CICADA_SERVER` and `CICADA_TOKEN` in the
tray's environment take precedence over the picked context as they do for `cicada`. Refresh cancellation does
not wait for the Tokio task to finish. A closed window discards late results.

## Blocking

None.

## Status

The app guide text is proposed. The docs link and guarantee tests and guide doctests passed. The feature builds,
dependency-tree checks and gate policy tests passed. The manual AppKit checklist is pending.
