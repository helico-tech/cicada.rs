# Proposal

## Why

In the owner's proposal for `inspector-actors`, a parent row of the Actors table folds open to show its children
(`/children`). Its design review found that folding needs its own paging per parent and stable row identities, and it
was split off so `inspector-actors` fits five tasks (2026-11-27).

## What Changes

- The Actors table becomes an outline: a row folds open to show its children (`/actors/{type}/{key}/children`), read
  when it is opened, with its own cursor and a "more…" row per parent.
- Children come as addresses: their rows show the address, type and running status; the rest is in the pane when one
  is selected.
- Rows have stable identities, so an actor shown both at the root and as a child keeps its selection and fold state
  apart, and a reload keeps the folded rows that still exist.

Decided in the design: identities by path, one read per parent with its own cursor, no per-child reads, and fold
state kept across a reload by a stated rule.

## Order

After `inspector-actors`, before `inspector-messages`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the outline and its model). Tests against an in-process server; a manual check list. Docs
handoff: the app guide.
