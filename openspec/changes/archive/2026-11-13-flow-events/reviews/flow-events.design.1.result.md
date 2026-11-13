Verdict: not yet

Blocking finding:

1. Publication IDs are not always legal run keys. `openspec/changes/flow-events/design.md`, “Atomic create and start,” requires the envelope ID as the raw key; the corresponding specification promises creation for each subscriber. However, `MessageId::new` (`crates/cicada/src/store/model.rs`) accepts `.` and `..`, while `ActorAddress::of` (`crates/cicada/src/address/mod.rs`) rejects both, as ADR 0021 requires. Neither the design nor scenarios specify this failure, which occurs before create_raw/send_raw and therefore is not covered by their refusal rule. Settle the policy before tasks: the smallest change preserves raw-ID identity and explicitly refuses these publications permanently when subscribers require a run address, with no effects. Add scenarios for both IDs, the empty-subscription case, and recovery using a fresh publication ID. Alternatively choose and document a collision-free key mapping for every accepted ID.

Notes:

- Add identity examples containing `/` and `%` to ensure construction percent-encodes the raw ID once rather than parsing it as an address suffix.
- Runner, sleep and batcher implementations are not present in the main checkout. Reconcile against their landed APIs and fixtures as the design already requires; the described continuation and depth contracts otherwise align.

Questions only the owner can answer: none beyond selecting the ID policy above.

Checks and limits: Read the change, adjacent proposal, dependency designs, architecture/ADRs, relevant core/actor code and existing specifications. OpenSpec list/show and strict validation passed. No test suite, implementation, generated contract or live database behavior was exercised. No following proposal exists in the current order.
