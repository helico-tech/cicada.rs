Verdict: not yet

## Blocking

1. Decisions 1–3. The root seam is sound. Use the same address seam for children because inserts and deletes shift page boundaries. Merge subsequent pages using the new cursor and drop tails at exhaustion. Prune removed path prefixes from caches, folds and pending reads before validating selection. `contains` checks only the immediate parent.

2. Decisions 2–3. Capture ordered actor paths with `rowsInRect` and `isItemExpanded` from the rendered outline. Exclude remembered folds under collapsed ancestors, define distance, ties and status-only viewports, and revalidate queued paths after ancestor updates. Define path-level stale marks for skipped, closed, failed and timed-out caches and when fresh reads clear them.

3. Decisions 1–3. Serialize refresh with root paging and each parent's More/Retry, including paging started mid-refresh. Bound waits by the deadline. Define Retry preservation and whether busy parents consume the 20 slots.

4. Decisions 1 and 4. Refresh the running-turn sample so badges stay current. Include it and all five pane reads in the shared four-request limit and deadline. Fence overlapping pane reads, keep each value alongside its sanitized error, and preserve the actor-missing behavior on 404.

## Notes

Five tasks fit with shared refresh machinery. Group root/scheduling, children, pane, AppKit, and handoff/checks. Inject viewport paths and delayed HTTP replies. Add a spec scenario where deleting an ancestor clears a selected grandchild. Manually check viewport capture, scroll restoration and stale marks.

OpenSpec validation passed. Tests and AppKit runtime checks were not run.
