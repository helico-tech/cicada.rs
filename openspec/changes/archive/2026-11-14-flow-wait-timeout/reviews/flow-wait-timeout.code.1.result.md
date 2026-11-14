Verdict: fix needed

Blocking findings

1. `crates/cicada-actors/tests/flow.rs:2883`: the agreed failed-decision commit case remains untested. `Op::Crash` unwinds out of `Flow::run` before `FlowActor::handle` processes `Latch::Close`; consequently neither the closed state nor its continuation reaches completion. This proves an aborted execution, but cannot establish the design's “If a decision commit fails” behavior or task 3's fault/rejection coverage. Add a focused completion-failure/rejection test after the runner has prepared timedOut plus its immediate continuation. Assert that the persisted wait/state version remains unchanged, the prepared continuation is absent, no timeout branch was returned, and an eligible reply can subsequently commit Received before another successful timeout decision. The public Worker/TurnSource seam is available: existing flow tests use Worker, and `crates/cicada-client/tests/remote_worker.rs:189` demonstrates a completion-intercepting source. No public Backend exposure is necessary. Keep the panic test as additional coverage.

Notes

- Deadline bounds/rounding, saved-duration replay, latches, mailbox ordering, recovery, metadata and immutable outcomes match the design by inspection. No other blocking finding.
- The contract fixture matches the predecessor after removing descriptions. Accepted the authorized description corrections and docs-session handoff; version metadata is 0.18.2.

Not checked

No tests, builds, Clippy or formatting checks were rerun. Assessed the builder's reported finishing checks; per-feature builds and Docker E2E remain release checks. Read the diff, agreed artifacts, relevant architecture/ADRs, tests, example and handoff; used OpenSpec show and a read-only fixture comparison.
