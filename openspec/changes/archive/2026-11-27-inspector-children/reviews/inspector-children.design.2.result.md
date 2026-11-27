Verdict: ready for tasks

All three blockers are fixed. D2 has one request marker per parent, "more…" disabled in flight, and closed rows keep children (reopen has no read; Refresh re-reads). D4 defines "forgotten" (a page passed the address, or the list ended) and ties the prefix rule to the debounced prefix. D1 removes map entries only after reloadData, and D4 re-opens through expandItem under the existing `syncing` guard. All notes are handled: no per-child reads, cycles explained, "—" outside the sample, selection as a path, wants_more on root rows, "No children.", the search rule.

Notes (none blocking):
- An open identity under a closed parent is never reached after a reload, so it is not forgotten until the parent is read again. Harmless; say it or let the next prefix change clear it.
- The code's `render_list` and `wants_more(last)` take addresses and indexes today; the tasks should name both.

Not checked: AppKit behaviour at runtime.
