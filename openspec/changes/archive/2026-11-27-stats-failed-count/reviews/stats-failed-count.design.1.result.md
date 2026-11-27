Verdict: not yet

## Blocking

1. Visibility rule stated wrong (design.md Decision 1; proposal What Changes "still kept (within retention)"). "Kept within retention, or still visible through visible_until" is the `retained` rule (postgres `retained!`, memory `is_retained`). The listing uses `visible`: when visible_until is set it alone decides (postgres `visible!`, SQLite CASE in `outcomes`, memory `is_visible`). They differ for failed outcomes within retention that are hidden: Keep::Remove (visible_until = finished_at, support.rs:443), Keep::For shorter than retention, Keep::Last trimmed. A builder following the text overcounts. Fix: define it as the listing's visible predicate without `since`; add a scenario and conformance case "a failed outcome hidden by its retention policy within store retention is not counted".

## Notes

1. PostgreSQL: `outcomes_failures_by_actor` serves the range and finished_at, but visible_until is not in it, so every kept failed row under the prefix costs a heap fetch. Bounded by retention plus sweep; say so.
2. PG read transactions are READ COMMITTED (store.rs:292): "same transaction" shares the store clock, not a snapshot. Proposal says "same query". Pick one wording.
3. `Option<u64>` makes `failed` optional and nullable in openapi.json though a 0.26 server always sends it; fine by the pinnedTo precedent (architecture.md). fixtures/ has no HTTP views: nothing to add.
4. StatsView lives in crates/cicada/src/api/mod.rs, not cicada-axum. Say whether the TUI header (tui/ui.rs) shows failed.
5. Consider `#[non_exhaustive]` on both structs so the next count is not BREAKING again.

## Owner questions

- ADR: AGENTS.md says a wire format change needs one. My pick: none, additive field.

## Not checked

No tests or query plans run.
