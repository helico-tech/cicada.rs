## Implementation

- [x] 1. Reconcile landed request/wait/latch APIs with the two predecessors; add Waited<T> and Step::wait_timeout.
  Parameterize the sleep wake-time helper's diagnostic kind without changing sleep behavior. Implement exact single/
  all/present-until replay matching, saved-duration precedence, new deadline validation before early replies, and
  New/Close/TimedOut/Waiting paths. Unit tests cover zero/1ns, MAX_DELAY and MAX_INSTANT boundaries, invalid duration
  with an early reply, ignored invalid duration on replay, handle/id/API misuse, 1000/1001, first latch, unpolled calls,
  typed decode errors and caught deserializer panic. Run the flow unit tests and public API doctests; verify inferred
  T and a Send flow future. Preserve existing indefinite wait and sleep tests.
- [x] 2. Extend runner handling to arm At(until) for new timed waits, close and arm now for decisions, and re-arm
  unchanged saved times before due. Reply turns close eligible timed single waits without consulting the clock;
  cancel older armed work without cancelling the current message. Keep joins/Cancel/cancelling refused. With
  start_paused/TestActors, prove open and closing effects are atomic, deciding turns run no code after the wait,
  Received/TimedOut data appears only on committed replay, late replies leave state/version unchanged even with a
  depth-62 value, and correlations and timer/immediate classification follow existing rules. Run targeted flow tests.
- [x] 3. Add virtual-time race/recovery tests for equal deliverAt in both seq orders, zero timeout with and without
  an early/queued reply, supplied open timed progress with a reply past until, a purged/cancelled/failed timer,
  public Continue normalization, deliver-now before the deadline, and pause/resume with both mailbox orders. Use
  existing fault/rejection harnesses to prove a failed decision has no committed branch and a reply can win before a
  later successful decision. Explicitly combine committed timeout, a transient failure in the following step, and a
  late reply before retry: it must still return TimedOut and not reissue the request. Check slow commit/clamping
  without rewriting the saved deadline; use controlled barriers, not sleeps. Run targeted tests plus existing
  request/reply and sleep recovery tests.
- [x] 4. Extend the predecessor's worked reply example with a match on Waited and a tested timeout branch. Compile
  the example and guide doctest with the real API. Compare regenerated existing flow and ready-made actor contracts
  against the landed flow-reply-waits predecessor: no schema/hash changes or new result wire format. Current main's
  pre-send snapshots cannot establish this equality. Run targeted contract tests and examples doctests.
- [x] 5. Update actors guide, architecture, ADR 0044 clarification, operations, public doc comments and changelog/
  version at build time. State claim-of-wait deadline origin, rounding, actual handling-order race, no reply cutoff,
  zero-timeout turns, immutable decisions, missing-timer recovery and no target cancellation/cleanup. Explain all-host/
  worker upgrade before enabling deadlines despite equal hashes. Preserve indefinite-wait guidance and deferred joins/
  finalizer scope. Run examples doctests and the docs test; no new guarantee row or migration is required.

## Finishing checks

- [x] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate flow-wait-timeout --strict`, with CICADA_PG_URL where available and skipped checks recorded.
  Confirm existing contract equality and request/send/wait/sleep behavior. The predecessors must land and archive
  before this delta; follow the build loop's normal review/release gates.
