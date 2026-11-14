## Implementation

- [x] 1. Reconcile with the landed send/reply/timeout predecessors, then add the four join helpers over one private
  wait engine and pure sufficiency/selection functions. Validate ordered slices, duplicates, phase references, reuse,
  1..100 size, shared journal room and new deadlines before early replies. Replay exact ordered ids/mode/deadline
  presence, allowing single/all one-request equivalence. Decode only saved selections under InProgress; keep permanent
  failure/caught panic semantics. Unit tests cover empty/101/duplicate/unknown/reused lists, first losers, 1000/1001,
  invalid new deadlines with replies, ignored new durations on replay, reordered/extended/mode-changed input, and
  single/all equivalence in both directions. Compile Send futures with Vec/array and a Send but non-Sync reply T.
  Run flow unit tests and API doctests; no iterator/builder API or new wire discriminator.
- [x] 2. Generalize Reply handling to sufficient all/first joins while preserving check order, terminal/reserved
  behavior, sender rules and value limits. Partial all replies keep armed/deadline/correlation; sufficient replies
  atomically save request-ordered selection, cancel older armed work and arm using the completing reply's correlation.
  Generalize New/Close/Parked/Waiting runner paths with no data returned before a committed decision. Keep unrelated
  early replies from changing the active wait. Test with start_paused/TestActors: partial and final all replies,
  parked waits, first winners, unrelated replies, no-flow-code counters, atomic failure, and metadata. Run targeted
  actor tests and existing single/timed wait regressions.
- [x] 3. Add virtual-time race/recovery coverage: both open and early first-arrival orders, earliest accepted reply
  independent of input priority, supplied array order, partial all timeout in both arrival orders, zero timeout with
  partial versus sufficient replies, supplied progress past deadline, last-needed reply/timer equal-time seq orders,
  late loser/missing replies (also too-deep values), pause and purged-timer recovery. Prove decision then later-step
  failure then late reply then retry preserves result positions and sends no request again. Test a decode error in
  the second selected value, caught deserializer panic and recovery after fixed code; losers/missing positions must
  never decode. Run the race/flow tests using controlled events and virtual time, not sleeps.
- [x] 4. Extend the worked examples and guide doctests with fan-out/all partial results and first-quote winner index.
  Keep external effects inside steps, and show a deterministic input list. Pin acceptance order versus request order
  and the interpretation of Vec<Waited<T>>. Compare existing generated flow/actor contracts against landed
  flow-wait-timeout: there must be no schema/hash diff. Use predecessor fixtures or a focused compatibility check to
  show its one-request all support and genuine join refusal; do not introduce a public feature flag or second runtime.
  Run contract tests, examples doctests and the virtual-time worked example.
- [ ] 5. Update ADR 0044 clarifications, architecture, actors guide, operations, public docs and changelog/version
  at build. Document stable input/result positions, reply acceptance order and restore-order preservation, closed
  losers still running, new cleanup requests for closed main requests, limits, partial timeout and typed decode
  recovery. Explain upgrade of every host/worker despite equal hashes and structural one-request equivalence.
  Keep cancellation/finalizer behavior deferred, with no core/store change or new G/S guarantee. Run examples
  doctests and the docs test.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate flow-wait-join --strict`, with CICADA_PG_URL where available and skipped database checks recorded.
  Verify actual contract equality and non-Sync compile cases against the implemented predecessors. Earlier flow
  changes must land and archive first; follow normal build-loop review/release gates.
