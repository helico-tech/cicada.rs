Verdict: ready for tasks

Round 1's blocking findings are fixed:
1. Paging under a filter: design decision 3 and the spec say no filter means paging on scroll; a typed filter means nothing loads by itself, one page per Load more, Retry after a failed page. The Tests section has the case of a filter hiding every row asking for no page, and the spec scenario matches.
2. The error scenario "A failed read" is in the spec.

Notes handled: Types Refresh and context switch, Watches Refresh, per-page filter text and placeholder, filter on watcher or target. The new claim that the (target, watcher) order needs no duplicate check holds: both stores use ORDER BY target, watcher.

Notes (not blocking):
- Say what happens when the filter is cleared again: scrolling paging resumes, one line in the design.
- The error scenario merges two failures; fine, but the builder should test both.
- A watches context switch is not stated (assume next visit, as for Types).

Questions for the owner: none.
Not checked: building or running anything; the docs blocks.
