---
id: watch-absent-target.code.2
kind: code
change: watch-absent-target
round: 2
base: 1c5f98603352332530224c7914422e7997cc3780
head: 7423ffa72d181cd9af8e46b578e3325eb8a8bf5d
previous: 6305fcfb1ad7c87b97dee99533f363eee967fa07
requested: 2026-11-25T15:44:10+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done watch-absent-target.code.2 <file>`.

## Note from the requester

Version: 0.22.0 (BREAKING)

Round 2 after a rebase with conflicts onto main 1c5f9860 (docker-context at 0.21.2). Check only the resolution: version
0.22.0 in Cargo.toml, Cargo.lock, plugin.json, both skills, docs/openapi.json, and nothing else of main's content lost
(skills/cicada/SKILL.md must keep main's container, permit-pool and job-graph routes); one CHANGELOG `0.22.0 — unreleased`
section holding docker-context's entries and this change's, with one summary and the BREAKING line; the reflow of
LedgerMsg in remote_worker.rs undone by doc comments on Failed/Stopped. Head 7423ffa7; workspace 3506 passed; examples
docs 11, skill 18, doctests 37; gate full and e2e pass.
