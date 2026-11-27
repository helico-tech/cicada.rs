Verdict: not yet

- `design.md` decision 1 and the spec’s “Duplicates across pages” describe an impossible scenario. All stores return one row per actor with an exclusive address cursor. Drop deduplication and test cursor continuation while leases change.
- `design.md` decisions 2–3 leave cache and read lifetimes unclear. Overview also calls `/nodes` on context switch. Specify a page-owned cache reused on return, emptied on switch, and loaded immediately if Nodes stays visible. Bound detail reads, invalidate them when filtering hides the selection, and Retry the selected id. Test late list responses across context switches and detail responses across A→B→A and Refresh.
- `design.md` decisions 1 and 3 leave Show Actor’s source history unspecified. Reuse the exact-address jump and define what Back restores from each link, including when a turn ended or its actor now runs another message.

Keep server health and absolute `last_seen`. Label cached health as of the last read. Five tasks fit with turns, node cache, node detail and history, AppKit, and handoff/checks. Strict OpenSpec validation passed. Runtime and AppKit checks were not run.
