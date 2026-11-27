# Docs handoff for inspector-soft-refresh-actors

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The Actors page's soft refresh re-reads the loaded root range, the first children page of the nearest open rows, the
running-turn sample and the selected actor's pane. It preserves open rows, selection by path and the first visible row
where they survive. The model tests in `crates/cicada-inspector/src/tests/soft_refresh/actor_lists.rs` exercise the page
bounds, pruning, scheduling, paging and retained pane data against an in-process HTTP server.

## 1. Soft refresh on the Actors page

**Where.** `docs/guide/app.md`, `Inspector on macOS`, with the live refresh description from `inspector-live`.

**Text.** The Actors page keeps open rows and the selection by path as it updates. When a row goes away, the inspector
removes its descendants and clears any selection under that path. The pane keeps showing each previous answer until its
replacement arrives. A failed pane read shows an error alongside the previous answer. The inspector refreshes the first
children page of up to 20 open parents, giving priority to parents nearest the visible area and re-reading ancestors
before descendants. A parent with a carried-over children page shows “not refreshed” on its **more…** or status row.

**Caveats.** One refresh uses up to four requests at once and has ten seconds. The root range can take up to three
100-row requests. The server returns full actor states for every root row fetched, up to 300 states per refresh,
including on the list that displays only actor metadata. Large states can make those requests expensive. Closed rows
and children beyond the refreshed range can retain older data. The user-facing live refresh description belongs to
`inspector-live`.

## Blocking

None.

## Status

The wording for the app guide is proposed. The macOS build, formatting, inspector and workspace Clippy, and strict
OpenSpec validation passed. The inspector passed 113 tests and one doctest. The workspace passed 3,998 tests with
PostgreSQL enabled. The standalone docs check passed 11 tests and the guide doctests passed 55.

The manual AppKit checks are outstanding, including scroll restoration, removal of open descendants, pane continuity,
the stale mark past the parent limit, keyboard navigation and VoiceOver.
