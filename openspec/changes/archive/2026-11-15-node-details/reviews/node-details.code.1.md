---
id: node-details.code.1
kind: code
change: node-details
round: 1
base: 44194a76da01ebcbff6040f2a73d829d8cd3b09a
head: d0d441dfe4e864395f6354d01c80460af1fd58dc
requested: 2026-11-15T18:20:03+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done node-details.code.1 <file>`.

## Note from the requester

Round 1 of node-details, a breaking release, 0.19.0. Head d0d441d on feat/node-details, based on 44194a7 (v0.18.12). Main has since gained only the docs session's 4c64a2b (docs/architecture.md, http guide, install, releasing), which this branch does not touch.

Gate: full checks and E2E passed on 9539dc4. d0d441d has a docs pass over that full pass (`python3 scripts/gate.py status d0d441d`).

The builder departed from the design in two places:
- `whoami` is added with `default-features = false, features = ["std"]` instead of plain `whoami = "2"`, which keeps the WebAssembly defaults out. Cargo.lock gains libredox, objc2-system-configuration and objc2-core-foundation. This is noted in ADR 0048.
- The app checks an explicit `--node-label` with its own copy of the three label rules (not blank, no NUL, at most 200 characters). A bad value is then a usage error (exit 2), and core needs no new public validation function. Please judge whether that duplication is acceptable or whether the copies can drift.

Other points to check:
- Migration 7 on both SQL stores. The upgrade tests from schema 6 include statements already prepared on an old connection. Rollback to 0.18 after the first 0.19 node opens the store is not possible; check that the changelog and handoff say so.
- Not tested: the host name lookup on platforms other than macOS and the Linux E2E containers. No test asserts the looked-up value.
