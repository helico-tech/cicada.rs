## Implementation

- [x] 1. Add the shared flow wire types and typed state checks from ADR 0044: Reply/Cancel, statuses, send/wait
  entries, replies and cleanup. Keep required JSON values distinct from missing fields, and schema bounds distinct
  from cross-field validation. Add unit cases for every invariant, including interrupted main/closed cleanup overlap,
  deadlines, reply references, duplicate ids, separate phase ids and terminal states. Preserve supported old step/sleep
  decoding without newly depth-checking old results. Regenerate the test and sample-service flow contracts with
  CICADA_UPDATE_CONTRACTS=true; check required-null fields and bounds in the actual schemas. Verify with the flow
  unit tests and targeted contract snapshot tests. Reserve later behavior explicitly; do not expose working wait,
  request or finalizer helpers in this piece.
- [ ] 2. Implement borrowed Target and Step::send with the specified inference, first-poll ordering, replay and
  outbox. Set InProgress before user Into/address/message/state conversion, preserve first-latch precedence, and
  refuse self targets. Test conversion counters and caught panics at each user-code boundary, unpolled futures,
  repeated/mismatched ids and request flags, capacity, and changed arguments on replay. Compile examples using &ref,
  Target::create and Target::child with a Send flow future. Keep the existing run/sleep tests green; run the targeted
  flow unit and actor tests. Distinguish caller argument construction/Drop from conversion and serialization.
- [ ] 3. Apply the outbox through ordinary Turn raw create/send operations, then save the entry and arm Continue
  in the same turn. Preserve node errors and retryability through the typed Store error chain; map Invalid and
  SendRejected permanently. With start_paused and TestActors, prove atomic create/send/progress, existing-state reuse,
  wrong-parent and missing-target commit rollback, recovery by Continue, registry failure retry, no replayed send,
  depth/NUL/null validation even for existing targets, and correlation/default retention/observer headers with no
  outgoing retry policy. Use the existing private fault harness where registry faults need it. Test reserved message
  refusals and terminal Continue behavior. Run the affected flow/runtime tests, including normal run/sleep recovery.
- [ ] 4. Add supported old-state samples and replay regressions, and an example of distinct old/new hosted type
  registration without pretending an upgraded host keeps the old schema. Add an EventActor regression that constructs
  FlowState::new as raw JSON, creates and starts the flow with the new defaults, and leaves Event's own contract
  unchanged. Verify old literal progress and required-null samples, event tests, contract hashes and example doctests.
  Do not add a generic export/restore utility or claim registration compatibility from successful state decoding.
- [ ] 5. Update architecture, actors guide, worked examples, operations, public doc comments and changelog. Keep
  ADR 0044 and its index consistent with the final implementation. Explain enqueueing versus target success, strict
  raw-send membership checks, replay argument rules, error policy, and the breaking contract/source changes (enum
  matches and FlowState literals). Document draining/archiving and pinned-old-binary deployment, the host-specific
  restore caveats, and upgrading every host/worker before enabling later equal-hash features. Select the breaking
  version at build time. Add relevant new proof tests to existing G2/G4 rows without inventing a new guarantee.
  Run cicada-examples doctests and its docs test; generated contracts and linked examples must agree.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, and
  `openspec validate flow-step-sends --strict`. Include PostgreSQL with CICADA_PG_URL where available and record any
  skipped database verification. Check regenerated flow contracts, unchanged Event contract and all existing flow
  behavior. Follow the build loop's normal review/release gates; this design changes no core or store schema.
