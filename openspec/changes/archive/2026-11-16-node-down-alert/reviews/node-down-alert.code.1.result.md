Verdict: fix needed

Blocking findings

1. `crates/cicada-actors/tests/node_down.rs:513` runs runtime behavior with real Tokio time, contrary to AGENTS.md's explicit paused-time/TestActors rule. The test keeps `web` heartbeating and assumes its next heartbeat cannot occur before the death assertions. A ten-second scheduling delay after `clock.advance_by(120s)` allows that heartbeat to refresh `last_seen`, making the expected alert disappear. Moving only the store clock does not remove this wall-clock race. Use deterministic paused-time orchestration; alternatively, keep the actual-row test focused on reader/no-registration and clean-stop behavior, since scripted TestActors tests already cover death detection. Update `openspec/changes/node-down-alert/docs-handoff.md:41`, which currently incorrectly describes all twelve integration cases as virtual-time scripted-source tests. Fixed means no wall-clock completion assumption and an accurate handoff.

Notes

- Sampling validation/caps, threshold, guarded scheduling, retained-watermark deduplication, read-failure recovery and Topic rollback agree with the design. Existing core guarantees and contracts remain unchanged.
- The actors guide is byte-identical to owner commit `7052813`, which contains only that protected path; its instructions match the source. Remaining prose is explicitly deferred under ADR 0045.
- The abandoned-claim test covers reclaim before execution, rather than loss after effects were computed; stronger replay coverage would be useful.

Not checked: I did not rerun Rust suites, PostgreSQL or Docker checks, or independently reproduce the reported gate runs. Review used head `2ea550f`, source/spec/test inspection, OpenSpec inspection and a clean `git diff --check`.
