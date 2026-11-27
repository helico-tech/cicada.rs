# Proposal

## Why

Live updates (`inspector-live`) reload the page on screen many times. A reload that drops the selection, the scroll
position or the open rows makes live updates unusable. Split from `inspector-live` by its design review (2026-11-27):
a bounded refresh that keeps the view is a change of its own.

## What Changes

- A soft refresh for the inspector's pages: it re-reads what is loaded, within limits on requests, concurrency and time,
  and puts back the selection, the scroll position and the open rows.
- Rows outside the refreshed range are kept and marked not refreshed, apart from rows the refresh found deleted.
- A change the user makes while it runs (a selection, a scroll, a fold, a Show Actor jump, a Retry) wins over the
  refresh's result.

To decide in the design: the limits (pages, open children, concurrency, time), and how each page restores its view.

## Order

After `client-change-stream`, before `inspector-live`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the refresh in each page's model). Tests on virtual time and against an in-process server;
a manual check list for scroll and folds. Docs handoff: the app guide.
