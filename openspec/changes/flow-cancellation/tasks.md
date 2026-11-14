## Implementation

- [x] 1. Reconcile the landed four predecessors, then implement status/message dispatch before normalization and
  the running Cancel branch with the current-message armed-id guard. Keep input/journals/replies untouched, skip flow
  code/input clone/extra result decode, and never write state on terminal no-ops. Tests use start_paused/TestActors
  and counters for never-started, armed, sleeping, missing-timer and permanently mismatched runs; repeated Cancel and
  done no-op; all messages in reserved cancelling; and trusted progress with armed=current id. Run targeted flow
  tests and existing send/sleep/wait regressions. No new API, permission check, self-purge or automatic target control.
- [x] 2. Add virtual-time wait/race coverage: parked wait, timed join with partial replies, too-deep late replies,
  both equal-delivery orders of wait closure versus Cancel, queued public Continues on both sides, last helper before
  done, a long handler held by a controlled future, retry backoff, pause/resume and purged Cancel. Assert retained
  journal/value equality and no normal work after committed cancellation. Test known unapplied commit atomicity and
  unknown acknowledgement separately with the existing fault harness: reread/resend idempotently, never assume an
  unknown commit rolled back or invokes handler retry. Distinguish lease/claim-limit recovery from sender retry.
  Run targeted race/fault tests and existing join/timeout recovery tests; use barriers rather than elapsed sleeps.
- [x] 3. Test ordinary control/observation boundaries: deleted-run send rejection, queued Cancel purged by delete,
  create-plus-Cancel, successful Cancel observed as done, no synthetic watch notification, no turn for cancelled
  armed work, retained target/child actors, outside/arbitrary actor sender and an ordinary best-effort change hint.
  Update public Cancel/status doc comments and regenerate flow snapshots if descriptions change. Compare hashes and
  description-stripped schemas against landed flow-wait-join; other ready-made/Event contracts stay unchanged.
  Run targeted control, flow and contract tests; generated descriptions are committed normally.
- [ ] 4. Update actors guide with a compiled Cancel example and the table separating flow Cancel, pending-message
  cancel, operator running-turn cancel, pause, purge, delete and Continue. Update architecture, operations, ADR 0044
  clarification and changelog/version at build time. Explain enqueue versus completion, lack of preemption/cleanup,
  retained terminal progress, earlier effects, status reads/hints, all-host/worker upgrade and safe resend/retry/redrive
  after predecessor refusal. State that already-cancelled runs are not finalized retroactively and resources need
  application cleanup until the next piece. Run examples doctests and the docs test. No new guarantee or migration.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate flow-cancellation --strict`, with CICADA_PG_URL where available and skipped checks recorded.
  Verify hash equality/description-only differences against implemented predecessors. They must land/archive first;
  follow normal build-loop review/release gates without changing core/store behavior.
