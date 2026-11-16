Verdict: fix needed

Blocking findings:

1. `examples/examples/pinned_process.rs:105`: post-start errors bypass the entire cleanup block. A store/decode error from `show_changes` while a child runs returns from `serve` immediately; startup/reconciliation/send errors use the same early-return pattern. Children fall back to runtime/handle drop without confirmed reaping or observation flush. Route every post-start exit through host cleanup, bounded system stop, pump join and store close, retaining the original error and reporting cleanup uncertainty. Add an injected-error test with a live child.

2. `examples/src/pinned_process.rs:552`: Start's desired-Stop refusal returns before the node/host-epoch check. A nonterminal run with desired Stop and observed Stopping (for example, an unconfirmed stop before host death) handled by a new host remains Stopping instead of NeedsReconciliation. The restart test sends Refresh first, masking this path. Check mismatches before this early refusal while preserving confirmed terminal states; test Start directly against this old-epoch state, asserting reconciliation and zero process actions.

Notes: The shared OS adapter, existing Tokio feature and shorter OS-test cleanup budget are reasonable departures. Reservation/tombstone retention and the reviewed lost-commit paths preserve one spawn per host slot. Version pins, generated-reference scope and documentation ownership match the brief. Gate status accepts fcba7e4 via the recorded full/E2E evidence for df9a5c5 plus its docs pass.

Not checked: no tests rerun; no Linux execution, manual demo execution or abrupt-host-death cleanup validation. Reviewed head artifacts, implementation, scenario tests, handoff, ADRs and gate status.
