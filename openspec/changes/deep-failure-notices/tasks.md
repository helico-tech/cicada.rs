## Implementation

- [x] 1. Enable serde_json float_roundtrip in the workspace dependency. Add conformance cases under
  crates/cicada/src/tests/conformance for numeric state/message reads and a failed depth-64 payload whose complete
  depth-65 notice is claimed. Cover the regression float and its negative, minimum subnormal, maximum finite f64,
  signed zero and i64/u64 limits; compare float bits and explicitly pin PostgreSQL JSONB normalization separately.
  Add local runtime and cicada-client HTTP Worker tests in the existing deep-value/remote-worker harnesses: a typed
  watcher compares the deep payload through a probe and stores only shallow metadata. Run targeted cicada conformance
  and runtime tests plus the affected cicada-client test target, including PostgreSQL when configured. Check the
  feature in a temporary consumer outside the checkout so workspace dev-feature unification cannot hide its absence.
- [x] 2. Add and re-export LetterPayload with value(), change DeadLetter.payload, and implement the private wire
  conversion/schema with exactly one raw payload or string payloadJson. Track raw null as present; avoid flatten or
  untagged buffering over deep data. Add the bounded depth walk, raw<=61/text>61 recording and redrive error handling
  without decoding text on state reads or adding live repair. Extend dead_letters.rs tests for depths 61/62/64,
  empty containers and mixed nesting, scalar/null and old/new marker-shaped data, exact numeric text conversion,
  malformed/parser-too-deep/depth65/NUL text, subsequent Discard and another letter's successful redrive. Keep existing
  capacity, once-only redrive, metadata, store errors and absent-target commit behavior. Prove serde/schema parity,
  including HTTP 422 for both/neither/null-text fields, and compatibility with old raw state samples. Regenerate the
  dead-letter contract, assert its hash changes, and update affected public-field uses. Run cicada-actors dead-letter
  and wire-format tests and the targeted HTTP validation test; ordinary depth limits and other watcher hashes stay.
- [x] 3. Add a tested Rust export-state conversion example using exact JSON parsing, with no lossy intermediate
  tool. Preserve every letter and metadata, converting readable legacy raw payloads to text when needed for restore;
  validate converted states against the new schema/limits before any deletion. Test old raw null/markers/floats,
  readable deep legacy data, malformed exports and already-redriven letters. Exercise the documented maintenance
  sequence on multiple independent childless instances: export, stop hosts, reset registration, restore states,
  pause flags and outgoing watches, then resume and observe a new failure. Demonstrate new instance identity and
  refusal of an old contract; cover preflight rejection of parent/child/incoming-watch topology and incomplete data
  in the example's checks where automated. Document the manual quiescence/backup obligations explicitly. Run the
  example's tests/doctests and affected actor/runtime tests on virtual time; do not operate on a real user database.
- [x] 4. Add a batch/event boundary scenario proving an accepted failed publication is retained and redriven
  unchanged while its flow state can fail again for the original depth limit. Confine the two independent repairs
  to their proposal scope: correct JournalEntry::Sleep's shipped behavior comment and regenerate description-only
  test.sign-up/sample-service flow fixtures with unchanged hashes; fix the event lifecycle test to execute
  Unsubscribe, assert its done outcome/subscription list/surviving runs, then delete and finish those runs. Correct
  the debouncer witness comment to identify state validation, not notice decoding. Run the affected event, debouncer,
  flow-contract and sample-service snapshot tests with CICADA_UPDATE_CONTRACTS=true only for regeneration, then verify
  with regeneration disabled. Do not broaden these repairs into runtime or flow changes.
- [ ] 5. Update architecture, ADR 0043/index if implementation wording requires it, core and actors guides, operations,
  public API docs and changelog. Explain encoded-state visibility/value(), existing batch/event limits, no automatic
  legacy repair and exact parsing versus database normalization. Write the maintenance instructions from the tested
  example: all actor names, verified backup/export/restore, quiescence, stopped local/remote hosts/bootstrappers,
  registration order, pause/watch restoration, unsupported topology, new instance identity and notification gaps.
  Never advise retrying business work as a lossless letter backfill. Add appropriate new proof names to existing
  docs/guarantees.md rows rather than inventing a new guarantee. Select the next breaking workspace version when
  built (expected 0.17.0); no SQL migration. Run examples doctests and docs tests and verify generated contracts.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate deep-failure-notices --strict`. Run the PostgreSQL conformance and numeric/notice cases with
  CICADA_PG_URL, recording any unavailable checks. Confirm exact parsing in the isolated consumer, run the upgrade
  rehearsal against disposable storage, and verify core/HTTP boundary tests run on ordinary debug runtime stacks.
  Check that only the dead-letter contract hash changed and the flow snapshot edits are description-only. Follow the
  build loop's review and release gates; this task list does not authorize a production migration.
