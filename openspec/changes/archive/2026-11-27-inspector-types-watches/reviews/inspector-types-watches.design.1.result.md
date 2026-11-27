Verdict: not yet

Blocking

1. Paging under the filter is undecided (design.md decision 3, proposal). "Paging goes on while the filter hides rows" and "Load more always offered" can be read two ways. Today paging is driven by scrolling: `wants_more(last)` is `last + NEAR_END >= rows.len()`, and `last_visible_row()` falls back to 0. If the Watches table counts only the rows the filter shows, a filter that hides most rows asks again after every page and reads the whole watches table with no pause. That ends only when the store runs out, and the rows stay in memory. A "Load more" button is also new, since today's footer says "more load as you scroll". Decide: while a filter is typed, no automatic paging, one page per Load more click; with no filter, scrolling as on Messages, and say whether Load more is shown then too. Add a test that a filter hiding everything asks for no page by itself.
2. An error path without a scenario (spec.md). A failed `types` read (Retry, no old list kept) and a failed watches page while no row is visible (Retry offered) are in the design's Tests but not in the spec. Add one scenario.

Notes

- Types: say what Refresh does to the selected type and the chosen schema (I would clear them, as Messages does). Say whether a context switch reads at once or at the next visit (Messages waits for the next visit).
- Watches: Refresh starts paging again and clears the selection. Say so.
- Each page keeps its own filter text and tooltip in the shared search field ("Address prefix" today).
- Owner question: should the filter be by watcher, or by target ("who watches X")?

Not checked: building or running anything; the docs blocks.
