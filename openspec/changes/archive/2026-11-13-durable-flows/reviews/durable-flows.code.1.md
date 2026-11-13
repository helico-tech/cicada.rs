---
id: durable-flows.code.1
kind: code
change: durable-flows
round: 1
base: d6f06b522da389cf6eb344fa2be0bf5b740d1e12
head: a081670a779e32bc268af8277cbddac8019cc704
requested: 2026-11-13T14:27:52+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done durable-flows.code.1 <file>`.

## Note from the requester

Version: 0.16.5

Builder report (round 1)

durable-flows (0.16.5) is built and every task is ticked, on branch feat/durable-flows in 7 commits. Nothing pushed, merged or tagged.

Tasks
1. Done: the API, ordered replay, the latch with its "in progress" marker, result checks and the armed continuation, in crates/cicada-actors/src/flow.rs.
2. Done: 22 virtual-time tests in crates/cicada-actors/tests/flow.rs, with a probe that counts every closure call.
3. Done: the test flow's contract pinned in crates/cicada-actors/tests/contracts/test.sign-up.json (the sleep entry included), plus state samples and FlowState::result checks.
4. Done: examples/src/flow_onboarding.rs and its tests. The sample service hosts flows.onboarding with no run at startup, plus its snapshot, text hash and a state sample.
5. Done: the "Flows" section of the actors guide with doctests, plus README, architecture, AGENTS map, crate docs, changelog and OpenAPI version.
6. Done: finishing checks.

Checks
- cargo fmt --all --check: clean.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2235 passed, 0 failed.
- openspec validate durable-flows --strict: valid.
- Not run: the Docker end-to-end suite.

Departures and open questions
- Replay mismatches, invalid or repeated ids and decode errors also lock in a permanent failure, so flow code cannot hide them by catching the error. Follows from the spec's "fails permanently"; the design text spelled it out only for result checks.
- futures added as a dev-dependency (tests catch panics with it); Clone derived on FakeOnboarding.
- Open question (raised with the owner in parallel; reviewer: say whether it conflicts with the design): the run ref carries the flow's full type, generics included (FlowRun<OnboardingFlow<S>>), so a producer must name the service type to send to a run.

New concepts
- Flow / Step: async code that runs again from the top on every execution. Each step.run returns a saved result or calls its closure once. A new result is saved together with the next Continue.
- Armed Continue: only that one message runs flow code. Sending Continue yourself starts or recovers a run, and resets its attempts.
- Step key: <run address>/<step id>, the same on every attempt. Calls happen at least once, so the external service must deduplicate by this key.

Breaking changes: none.
