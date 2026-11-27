# Proposal

## Why

In the owner's proposal for `inspector-actors`, a parent row of the Actors table folds open to show its children
(`/children`). Its design review found that folding needs its own paging per parent and stable row identities, and it
was split off so `inspector-actors` fits five tasks (2026-11-27).

## What Changes

- The Actors table becomes an outline: a row folds open to show its children (`/actors/{type}/{key}/children`), read
  when it is opened, with its own cursor and a "more…" row per parent.
- Children come as addresses; their type, version and status are read for the visible ones.
- Rows have stable identities, so an actor shown both at the root and as a child keeps its selection and fold state
  apart, and a reload keeps the folded rows that still exist.

To decide in the design: how a child's details are read (one call per visible child, or a batched read), and how a
reload keeps fold state.

## Order

After `inspector-actors`, before `inspector-tables`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the outline and its model). Tests against an in-process server; a manual check list. Docs
handoff: the app guide.
