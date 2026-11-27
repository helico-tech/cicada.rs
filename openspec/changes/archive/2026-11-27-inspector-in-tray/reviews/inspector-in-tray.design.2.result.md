Verdict: ready for tasks

All four findings are addressed. Every quit path closes the inspector and guards against reopening. Both modes share teardown, fence late results by generation, and abort refreshes on the supplied runtime without joining. Settings-style focus, window-local ⌘W/⌘R, menu-scoped ⌘I, and the shared context fallback are specified.

Notes for tasks

- Set the quit flag before closing even on immediate AppKit termination. Release mutable borrows before calling AppKit.
- Test Settings editing and one refresh with the toolbar item in overflow. Exercise Connecting and Stopping, server-only resolution, and malformed configuration.
- Require the inspector edge present with the feature and absent by default. Run feature-on check, clippy, and tests on macOS.

Strict OpenSpec validation, baseline dependency checks, and 69 feature-off tray tests passed. Feature-on tests and dependency checks could not run because the feature is unimplemented. GUI checks were not run.
