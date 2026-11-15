---
id: local-push-gate.code.2
kind: code
change: local-push-gate
round: 2
base: 48959e3d979534985bb13e255871b66d13aed730
head: 33a665f20711f47954c7cdcbd7331b2e4a0cac2b
previous: 18ffa1bf25177b7e5bf42bc64f93c783b697001e
requested: 2026-11-15T09:45:45+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done local-push-gate.code.2 <file>`.

## Note from the requester

Version: 0.18.9

Round 2: fixes the two round 1 blockers (range 18ffa1b..33a665f: f4a62fc fixes both, 38046e6 fixes a lock-test race the post-fix gate run caught, 33a665f records results in the handoff). Round 1's report is .reviews/local-push-gate.code.1.result.md.

Builder's report:

Finding 1: a new `supervised()` runs each command in its own process group that inherits the lock, is recorded as the marker's child, and is stopped and reaped on INT/TERM; output goes through temporary files so a leftover process holding a pipe cannot block the gate. The release runs release-check, skill packaging, every gh call and the git calls that talk to the remote (ls-remote, fetch, main push, tag push) through it; the tool-version probes too. Kill-then-recover test: a fake gh sleeps in `release create`, the gate is killed; gh survives and holds the lock, so recover and a second release say busy; after gh ends a retry reports the run abandoned, recover clears the marker, and the release resumes and publishes. TERM test: TERM during draft publication stops gh, the draft stays unpublished and the marker stays until recover, after which a retry publishes.

Finding 2: execute_kind writes a running record before anything fallible; a normal failure, including a refusal during preparation, records failed; an interruption leaves running. `run --force` marks every record it will replace unusable at the start, e2e included with --e2e. Tests for full, docs and e2e: a Cargo config refusal leaves the record unusable and unauthorized; an empty PostgreSQL URL leaves it failed; TERM during a blocking `rustc -V` leaves it running, then recover works.

Lock-test race: the tests waited for any child, which now includes the rustc probe; they wait for the check command itself, and the helper keeps polling when macOS answers EPERM for a group holding only an unreaped zombie.

Checks: 64 Python tests pass directly and inside the gate; fmt --check and clippy -D warnings clean; doctests, --test docs, --test skill pass 47. Gate on 38046e6 (temporary clone): full checks passed, 3055 Rust tests with PostgreSQL, 0 failed; all feature-alone builds and dependency-tree checks; 28 E2E passed. On 33a665f fresh docs checks over that full pass, E2E reused. Safety re-checked: no gate directory or pre-push hook in the shared repository; no push, tag or release.
