Verdict: fix needed

## Blocking

1. **A row moved ahead drops many rows**: `crates/cicada-inspector/src/pager.rs:387`. The seam is now the furthest old row the range holds. Messages are sorted by `actor_address, deliver_at, seq`, and Deliver Now sets `deliver_at` to now (`sqlite/store.rs:2016`, `postgres/store.rs:1837`). So one deep row can jump into the range.
   Scenario: one actor has 450 loaded messages, m000..m449. Someone uses Deliver Now on m440. The re-read returns m440 and m000..m298. The seam lands after m440, so only m441..m449 are kept. Rows m299..m439 (141 still-pending rows) vanish. Paging brings them back later, but after the tail. I proved it with a scratch test: "missing 141 rows: 299..439". The old code lost only m299 here.
   Fixed looks like: a row that moved ahead does not decide the seam. For example, keep every old row the range does not hold, marked "not refreshed". Add a test that moves a deep row to the front.

## Answers

- Outcomes: sorted by `finished_at, message_id`, and rows do not move. The fix is right there.
- A consumed row before the seam goes, as it should. A row rescheduled later after a failure leaves the list until paging reaches it, then lands at the end. The old code did the same. This is a note, not a blocker.
- The changed deadline expectation fits decision 2. When the range holds no old row, every old row may be past it.
- The new test fails on the old code: 450 rows, not 455. I checked this in a scratch copy.
- None of round 1's notes should block.

## Not checked

I ran the `cicada-inspector` tests at head: 95 passed. Not run: the workspace, clippy, fmt, the gate, AppKit.
