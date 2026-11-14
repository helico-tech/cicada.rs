## Implementation

- [ ] 1. Reconcile the landed flow-step-sends APIs, wire, checks and outbox with this design before extending them.
  Implement Request<T>, ReplyTo<T>, the reply helper and Step::request. Keep route schemas/traits independent of T,
  private request construction, borrowed Target inference, and all user conversion/build/encode work behind the
  request's InProgress marker. Compile examples with &ref/create/child and Send flow futures. Prove the reply helper's
  future borrows neither route nor value (use precise opaque-future captures as needed), encodes before return and
  needs no T:Sync bound. Unit tests cover conversions/builders skipped on replay/after latch/unpolled, caught panics,
  mismatches, self targets, entry 1000/1001, null/depth/NUL and encoding errors. Verify targeted flow unit tests and
  doctests; preserve Store error chains and the predecessor's atomic outbox/error policy.
- [ ] 2. Implement Reply handling in the runner without executing flow code. Check terminal/reserved phase, known
  request, sender, duplicate and closed status before value limits. Store early replies without changing armed;
  atomically close a supported open wait, cancel old armed work and arm with the reply's correlation. Preserve
  unsupported join/deadline refusal until their pieces land. Add start_paused TestActors cases for every ignore/fail
  path, state/version unchanged on ignores, first value wins, null, 61/62/63 depths, required value, target versus
  child/delegate/absent sender, no-flow-run counter, correlation and no outgoing retry. Include request child collision
  rollback/recovery and a registry fault through the existing harness where needed. Run targeted flow actor tests.
- [ ] 3. Implement Step::wait and the Parked/Close/new-wait latch paths. Commit a closed outcome before any typed
  decode or later flow code, including early and supplied replies; park with armed=null and no recurring message.
  Validate handle identity, unused earlier request, exact single/all/no-deadline replay shape and shared id/capacity
  rules. Protect custom Deserialize with the marker; ordinary decode failure and caught panic must block later helpers
  without losing the saved outcome. Test first-latch precedence, unknown/reused handles, 1000/1001 entries, early reply,
  public Continue normalization then parking, pause/resume, lost/extra continuation and recovery after fixed decoding.
  Use start_paused/TestActors; run the relevant unit/flow tests and existing sleep/send replay regressions.
- [ ] 4. Pin ReplyTo's required fields/address/id schema, equality for different T, and a new target contract fixture.
  Add a worked example of a job or approval actor replying later and a flow requesting/waiting; its Rust code compiles
  as an examples doctest. Compare actual generated contracts to the landed predecessor, not the current pre-send
  checkout: existing test/sample-service flows and ready-made actors must have no schema/hash diff. Regeneration may
  add the new target fixture but must not silently change existing contracts. Run targeted contract/snapshot tests,
  examples doctests and a virtual-time end-to-end request/reply example, including deletion causing reply commit refusal.
- [ ] 5. Update architecture, actors guide, operations, public API docs, ADR 0044 and its index, and changelog/version
  at build time. Explain sender-address trust (HTTP cannot supply from; trusted remote completion uses its actor),
  delegates forwarding through the target, first eligible value, indefinite waits on target failure, deleted-run commit
  rejection, early-reply correlation, committed outcomes and typed decode recovery. Document all-host/worker upgrade
  before enabling requests, matching-hash unsupported behavior, lost predecessor replies and changed sender on
  dead-letter redrive. Adding ReplyTo to a host target follows strict contract upgrades. Add proof links only to
  applicable existing guarantee rows. Run examples doctests and the docs test.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate flow-reply-waits --strict`, with PostgreSQL through CICADA_PG_URL where available; record skipped
  database checks. Confirm actual contract equality and existing flow/event/send/sleep behavior. flow-step-sends must
  land and archive first: its new requirements are the targets of this change's MODIFIED deltas. Follow normal
  build-loop review/release gates; no core/store or flow-wire change is expected.
