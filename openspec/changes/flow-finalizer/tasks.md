## Implementation

- [x] 1. Reconcile the five landed predecessors before editing. Add default-success Flow::on_cancel, public Cleanup
  wrapping/dereferencing Step, and phase-aware journal/key handling with the actual ActorAddress separate from the
  external key prefix. Running Cancel must commit cancelling/armed cleanup without invoking user code; repeated
  Cancel there is a state-write-free no-op. Executing Continue selects main or cleanup solely from status. Record
  cancelled only after successful hook return with no latch and all cleanup entries consumed. Unit/actor tests cover
  the full status/message matrix, default two-turn transition, done and old-cancelled no-hook cases, current-id guard,
  stale/public/redriven Continue, repeated Cancel not starving cleanup, same ids/distinct keys, self-target/ReplyTo
  addresses, early return/mismatch and entry 1001. Run targeted flow tests with start_paused/TestActors.
- [x] 2. Implement main_key/result/request/reply/journal and MainReply with stable main-state semantics. Fallible
  lookups must check the shared latch, validate ids/kinds/handle ownership, distinguish None from wrong kind, and
  protect custom decoding with InProgress/Permanent. They consume no ids/cursor. Keep raw journal reads documented
  as ordinary host code. Test absent/present/wrong-kind/invalid input, cleanup handles refused by main_reply, repeated
  lookups, post-latch calls, decode errors/caught panics followed by Ok, and stable Open despite arriving replies.
  Compile default and overriding hooks, helper resolution through DerefMut, sequential lookup/wait borrows and a
  Send future with a Send but non-Sync reply type. Run flow unit tests and API doctests.
- [ ] 3. Enable cleanup waits over earlier cleanup requests and unclosed main requests, including mixed joins;
  preserve the existing structural rules rather than tightening decoding. Route replies during cancelling only to
  cleanup, checking closed waits across both journals. Test interrupted open main wait plus closed cleanup wait
  with main byte-identical, and explicitly assert main_reply stays Open after that cleanup wait closes. Cover late
  replies blocked by cleanup closure, never-awaited main requests, closed main winners/losers, request reuse, early
  first acceptance order, partial all joins, cleanup-owned deadlines, zero timeouts and both equal-time Cancel/main
  reply or timeout orders. Use start_paused/TestActors; run targeted phase/join/timeout tests and prior main regressions.
- [ ] 4. Add recovery/observation and the worked resource-owning example with deterministic tests. Cover handler
  retry exhaustion, public Continue recovery, purged and parked cleanup, pause, known unapplied and unknown-ack commits
  at phase entry and final status, no main replay, and no synthetic watch notifications. Distinguish handler policy
  from lease/claim-limit recovery. Prove delete runs no hook while preserving ordinary core child-removal behavior.
  The example must confirm Stop before capacity release, keep effects in steps, release an external reservation by
  main_key even when the result never committed, and model idempotent owner fencing/tombstones against late creation.
  Test that a main reply arriving between cleanup turns cannot flip its Open-based branch, and that the final
  application notification commits separately before cancelled. Run example and targeted fault/flow tests.
- [ ] 5. Update rustdoc, ADR 0044 clarifications, changelog/version and any description-only flow snapshots. Compare
  hashes and description-stripped schemas against the landed cancellation predecessor; ReplyTo, Event and other actor
  contracts stay unchanged. Write docs-handoff.md under this change for the dedicated docs session, following current
  AGENTS.md ownership: give exact source/tests, actors-guide/API/example content, architecture phase rules, current
  upgrading/operations destinations, limits and rollout caveats. Include identical host-code requirement, old/default
  hooks skipping cleanup, lost replies not restored by Continue alone, no retroactive cleanup, stable lookup semantics,
  raw decode caveat and uncertain external effects. Do not edit owned handwritten prose. Run contract tests, examples
  doctests and docs checks; coordinate any required existing-doctest updates through the docs owner under the landed
  handoff procedure, without skipping release checks. No new guarantee, wire format or migration.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate flow-finalizer --strict`, with CICADA_PG_URL where available and skipped checks recorded.
  Verify actual compilation, retained main/cleanup overlap, stable lookup regressions and contract equality against
  implemented predecessors. They must land/archive first. Follow normal build-loop code-review/release and current
  docs-handoff gates; this completes the six-piece flow line without core/runtime/store changes.
