# Docs handoff for inspector-soft-refresh

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The macOS inspector preserves the first visible row across table reloads, including paging and manual Refresh. The
paged and cached pages also have an internal soft refresh that preserves the current selection and pane, including a
followed link in Outcomes. The model tests in `crates/cicada-inspector/src/tests/soft_refresh.rs` exercise the range,
selection, cancellation, deadline and stale rows against an in-process server.

## 1. Scroll position across reloads

**Where.** `docs/guide/app.md`, `Inspector on macOS`.

**Text.** Paging and Refresh keep the first visible row in place. If that row disappears, the table keeps the previous
scroll offset as far as the new table allows.

**Caveats.** Soft refresh is internal until `inspector-live`. The existing Refresh command still uses the page's
manual refresh behavior. A manual Refresh continues to clear the selected row where it did before.

## Blocking

None.

## Status

The scroll wording is proposed for the app guide. Formatting, Clippy and strict OpenSpec validation passed. The
workspace passed 3,979 tests with PostgreSQL, including 11 new soft-refresh tests. The standalone docs check passed
11 tests and the guide doctests passed 55.

A native AppKit smoke check exercised paging, a temporary return to the first page, deletion and the fallback to
the previous scroll offset. The model tests checked the stale-row mark. A visual walkthrough on each page, keyboard
navigation and VoiceOver remain for a manual run.
