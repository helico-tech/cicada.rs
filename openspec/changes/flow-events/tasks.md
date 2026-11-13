## Implementation

- [x] 1. Reconcile with the landed flow runner and batcher, then add EventActor/ref/state/messages/helpers and crate
  exports. Implement bounded validated subscriptions, Publish/PublishBatch normalization and atomic raw create/send,
  mapping address/call refusals to permanent errors and preserving store retryability. Add library tests for state,
  name/count bounds, helper serialization, missing fields and raw dot/slash/percent ids; run library tests and doctests.
- [x] 2. Add virtual-time TestActors integration tests for every spec scenario: multi-flow atomic starts, batching,
  empty subscriptions/batches, invalid dot ids with/without subscribers and fresh-id recovery, context and retry
  selection, hosted/unhosted invalid input, stale subscriptions, same-id retention boundaries, new-id redrive,
  scheduler/composition starts, independent lifecycle and depth. Include both encoded-looking and raw slash ids.
  Use public shared-store nodes and the existing registry-failure fixture as needed; run those targeted tests.
- [ ] 3. Pin Event's contract and all message/initial/populated/invalid state samples in wire-format tests and
  `docs/reference/contracts/cicada.event.json`. Host Event in the sample service without automatic instances or
  subscriptions. Regenerate references and sample-service snapshots/texts with `CICADA_UPDATE_CONTRACTS=true`;
  verify those tests and unchanged existing actor/flow contracts.
- [ ] 4. Add tested examples and actors-guide doctests for explicit subscriptions, general/batch typed input, event
  and flow watches/recovery, scheduler/limiter/debouncer Publish and batcher PublishBatch. State atomicity boundaries,
  input validation limits, dot-id refusal, encoding, fresh-id redrive, retention/key reuse and batch depth. Update guide
  tables, README, crate docs, architecture's crate row, AGENTS map and changelog. Verify examples,
  `cargo test -p cicada-examples --doc` and `cargo test -p cicada-examples --test docs`.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate flow-events --strict`; record results and any unavailable optional checks.
