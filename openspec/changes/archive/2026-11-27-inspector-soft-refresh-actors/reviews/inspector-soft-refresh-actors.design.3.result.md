Verdict: ready for tasks

Decision 3 schedules re-reads by depth, ranks visible rows first within each depth, and prunes paths removed by an ancestor's re-read before issuing their reads. Decision 5 sequences samples at request time and prevents older results from replacing newer ones. Decision 2 merges every children page by address, keeping rows sorted and unique.

Task notes. Align the “Many open rows” scenario with depth ordering. When More exhausts the cursor, test that retained children absent from the newly covered range are dropped.

Strict OpenSpec validation passed. Rust tests and AppKit checks were not run.
