---
id: containers-guide-doctests.code.1
kind: code
change: containers-guide-doctests
round: 1
base: a8d529308f96b963fd5d6a3b6d1faefa0e1a955c
head: d478d695eb82634003e558b11dd96ef31dba43c7
requested: 2026-11-25T17:16:57+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done containers-guide-doctests.code.1 <file>`.

## Note from the requester

Version: none (fast lane, tests only)

Builder report (head d478d695): the guide is in examples/build.rs, ContainersGuideDoctests in examples/src/lib.rs,
cicada-containers a path dev-dependency of cicada-examples with testing and docker (one Cargo.lock line). Checks: fmt,
clippy -D warnings, cicada-examples all suites (doctests 37/37), gate full pass of d478d695. Departure: the temporary
probe blocks went into the struct's doc comment, not the protected guide (reverted, not committed); a path dependency
as in e2e/Cargo.toml. Docs: handed off (AGENTS.md doctest list; the guide's "no compiled examples" line).
