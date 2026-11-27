# Proposal

## Why

Live updates (`inspector-live`) reload the page on screen many times. A reload that drops the selection or the scroll
position makes live updates unusable. Split from `inspector-live` by its design review (2026-11-27), and split again by
its own: the paged and cached pages here, the Actors page in `inspector-soft-refresh-actors`.

## What Changes

- A soft refresh for Messages, Outcomes, Watches, Running Turns, Types, Nodes and the Overview: it re-reads what is
  loaded within limits and keeps the selection and the pane.
- Every reload of a page's table keeps the scroll position.

## Order

After `client-change-stream`, before `inspector-soft-refresh-actors`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the refresh in each page's model, the scroll restore in AppKit). Tests on virtual time and
against an in-process server; a manual check list for scrolling. Docs handoff: the app guide.
