# Proposal

## Why

The inspector shows how many messages failed (the Overview tile and the Outcomes item in the sidebar; mockup:
https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH, page Inspector). The API has no such count: `GET /outcomes` returns
pages of finished messages, and counting them that way costs one call per page. The owner agreed to add the count
(2026-11-26).

## What Changes

- `GET /stats` gains `failed`: the number of failed outcomes that are still kept (within retention) under the
  prefix. With no prefix, every actor counts.
- The store answers it in the same query as the other numbers. Both backends pass the conformance suite.
- The client and `cicada stats` show the new number.

## Out of scope

- Counts of cancelled or done outcomes, or counts per time window ("today"). The inspector labels the number "failed,
  kept 7 days" (the retention).
- Any change to how long outcomes are kept.

## Order

After `tray-settings`, before `inspector-window`.

## Impact

Crates: cicada, cicada-axum, cicada-client, cicada-app.

`crates/cicada` (store query, both backends, conformance), `crates/cicada-axum` (`StatsView`), `crates/cicada-client`,
`crates/cicada-app` (`stats` output); `docs/openapi.json` and `docs/reference/cli.md` regenerated. An additive change to
the HTTP contract; the design says why it needs no ADR. Docs handoff: the HTTP guide and operations.
