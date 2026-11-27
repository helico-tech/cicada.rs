Verdict: ready for tasks

- Running Turns uses the exclusive actor-address cursor without deduplication and tests paging while leases change.
- Nodes owns its cache. First visit, its Refresh, and a context switch trigger reads, with hidden pages deferring until visited. Overview refresh is independent. Health is labelled “as of the last read.” Detail reads are dropped on selection changes, Refresh, and context switch. Retry targets the selected node.
- Show Actor uses the exact-address jump. Back restores the current turn or the saved ended row, and reloads the selected node’s detail.

Task notes. Verify turn status on Back with a fresh exact-address read. Cached or partial pages cannot establish absence. Reuse the read deadline and test late Node list results across switches, filter-hidden selections, and A→B→A.

Strict OpenSpec validation passed. Runtime and AppKit checks were not run.
