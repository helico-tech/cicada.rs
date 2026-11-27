# Proposal

## Why

The soft refresh of `inspector-soft-refresh` covers the paged and cached pages. The Actors page has more to keep: the
open rows and their children (`inspector-children`), the outline's path selection and its pane. Its design review split
it off (2026-11-27).

## What Changes

- A soft refresh for the Actors page: the root rows as the other paged lists, the first page of children of open rows
  within a limit, nearest the visible area first, the open rows and the selection by path, and the pane re-read
  without a flash.
- Children past their first page, and the children kept for closed rows, are marked not refreshed.

Decided in the design: the view reports the open rows on screen and the first visible row; the cost of full states is
capped at 300 rows and named in the handoff.

## Order

After `inspector-soft-refresh`, before `inspector-live`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the Actors model and outline). Tests against an in-process server; a manual check list.
Docs handoff: the app guide.
