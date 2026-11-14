Verdict: fix needed

1. `crates/cicada-actors/src/flow.rs:1585` — “Reserved active wait” has no test. The join/first/deadline cases in `tests/flow.rs:1783` exercise Continue replay, never an eligible Reply through `accept`. Add virtual-time actor cases sending a valid reply from the saved target into each reserved open shape; assert permanent later-feature refusal, unchanged state/version, no saved reply/outcome, no new continuation and no flow execution.

2. `crates/cicada-actors/tests/flow.rs:1941` — “Sender boundary” is incomplete: no flow test completes the target’s leased turn through a remote worker and verifies that its Reply passes FlowActor’s sender check. The report explicitly omits this scenario. `cicada-axum/tests/leases.rs:226` proves server-derived sender for a generic Add, but does not exercise the agreed request/reply integration. Add that case and verify the accepted value and closed wait; cover dead-letter redrive rejection as the same scenario requires (currently only delegate/child/outside sends are exercised).

Notes: No implementation defect found in the inspected request, reply, wait, rollback and recovery paths. The early-reply correlation test uses the same correlation throughout; distinct reply/closing-Continue correlations would make that assertion meaningful. The existing flow fixture is unchanged after removing descriptions; the authorized description corrections and docs handoff are consistent with the code.

Not checked: no test suite or Docker E2E run. Assessed the builder’s reported fmt, clippy, workspace/PostgreSQL and OpenSpec checks as reported; did not independently reproduce them. Reviewed base 8605589 through head 3fd6aea, artifacts, relevant ADRs, tests, example and version changes.
