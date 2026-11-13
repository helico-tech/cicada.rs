---
id: debouncer-actor.code.2
kind: code
change: debouncer-actor
round: 2
base: 16cc667671a280bdfa4836ca7a69d4672445ba84
head: 965d9901b21a90e8b2ccda6585a49e83bc5fbb99
previous: 13743afde864a582526eb479bce9cee35062391c
requested: 2026-11-13T10:33:54+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done debouncer-actor.code.2 <file>`.

## Note from the requester

Version: 0.16.3

Builder report (round 2: fix of round 1)

Blocking finding 1 of round 1 is fixed in 965d990 on feat/debouncer-actor; nothing else changed.

The fix (an_old_failed_flush_redriven_flushes_the_new_burst_now in crates/cicada-actors/tests/debouncer.rs):
- The first flush fails because worker/c is absent; the test asserts the dead letters keep it.
- The old burst ends with a manual Flush once worker/c exists: c gets "old" and the state clears. The dead letter stays.
- At 12 s a fresh burst starts. Before the redrive it asserts firstAt is 12 s, the new timer is due at 14 s (still in the future), and "new" has not been delivered.
- After the redrive it asserts "new" is delivered at 12 s (early), the state is cleared, the new burst's timer ends Cancelled, and the debouncer's mailbox is empty.
Without the redrive nothing is delivered (run_until_idle does not move time and the timer is due at 14 s), so the final assertions cannot pass by accident. The mutation itself was not run.

Checks
- cargo test -p cicada-actors --test debouncer: 23 passed, 0 failed.
- cargo fmt --all --check: clean.
- cargo clippy --workspace --all-targets -- -D warnings: clean (also a forced fresh run on cicada-actors).
- cargo test --workspace with PostgreSQL: 2149 passed, 0 failed.

Departures and open questions: none. The deep Failed-notice issue stays out of this change.
Breaking changes: none.
