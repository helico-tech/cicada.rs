Verdict: not yet

1. Decision 3 puts visible rows first but requires ancestor validation before descendant reads. A visible child can outrank its offscreen parent. Define dependency priority and how it counts toward 20.
2. Decisions 4–5 leave the initial `sample_task` unfenced. It can outlive root paging and overwrite fresh running badges under the unchanged generation. Reuse or cancel it before resampling.
3. Decisions 2 and 4 reset the cursor. More currently appends, duplicating retained children. Specify merging the newly covered range and pruning unseen tails at exhaustion.

Task notes. Serialize first expansion reads alongside More. Show stale marks and errors even on exhausted child lists. Test deletion of a selected grandchild’s ancestor. The shared five-read pane budget, per-part errors and missing-actor behavior are specified.

Strict OpenSpec validation passed. Rust tests and AppKit runtime checks were not run.
