---
id: placement-offers.code.2
kind: code
change: placement-offers
round: 2
base: ef1b6dc84f125eb1244addf0ce2db849eb8908cc
head: e27959d16cff34be305ef6ba28b692abf2486374
previous: 757adf4fef612c5202c0e393777d132c3b0ec245
requested: 2026-11-26T05:13:24+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-offers.code.2 <file>`.

## Note from the requester

Version: 0.24.0 (the placement line's release)

Round 2 (head e27959d1): the heartbeat watch's pattern. State field armedId (the armed Check's message id; armed and
armedId both set exactly while a node is live, else the decode refuses); only the armed Check counts, an early run
re-arms at the same time and marks nothing; other self-sent Checks change nothing; Rearm cancels the old Check first.
New tests: a Check made due early (deliver-now at +10 s, the node silent with one notice 600 s later); an operator's
retry of a cancelled Check is refused and changes nothing; three Rearms leave one Check. Contract JSON regenerated,
samples include armedId; rustdoc, handoff and changelog fixed. Note: an operator's retry of a Check arrives with a new
id and no sender and ends failed (refused), state unchanged. Checks: fmt, clippy, workspace with PostgreSQL 3648; gate
full pass of e27959d1.
