---
id: actor-ports.code.1
kind: code
change: actor-ports
round: 1
base: f659a339dc550e2cd2a765fb90bca9561aa91536
head: 7047227ae424082030b86bed839b1140b625e43c
built-by: gpt-6.2-sol
requested: 2026-11-28T14:07:37+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-ports.code.1 <file>`.

## Note from the requester

Version: 0.26.0

Builder's report (gpt-6.2-sol), head 7047227a on feat/actor-ports, fast lane:
- Tasks 1–4 done. Two virtual-time tests host the real cart with a fixed-price and a rejecting fake pricing actor, both `impl Actor` for the `Pricing` ref; a third test checks that both `PricingReply` variants decode as `CartMsg` (examples/tests/reply.rs:107).
- Docs handoff, no blockers; it carries the change loop's answer to the builder's question: the contract hash comes from the type name, message family and state schema, so a fake can share the real actor's hash while behaving differently; fakes belong in a test store.
- Checks: fmt, clippy, openspec validate --strict; full gate passed (4,107 workspace tests with PostgreSQL, 206 app tests, 127 Python tests, 9 browser-client tests, 0 failures); the final commit has a docs gate pass over the full pass of c381c44c. No E2E (not asked). No breaking changes.
- Example: `cargo test -p cicada-examples --test reply`.
