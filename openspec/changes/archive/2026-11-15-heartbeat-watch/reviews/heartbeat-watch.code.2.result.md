Verdict: pass

Blocking findings: None. Both round-1 blockers are resolved.

The schema-boundary test uses a non-hosting HTTP node on the hosting test node’s shared store. It verifies shape rejection (422, no instance), accepts the cross-field-invalid alive state (201), and runs Beat, Rearm and Expire through the hosting executor. All three fail with FailureCause::State; raw state/version and the timer/alarm mailboxes remain unchanged.

The eligible self-target test completes a publicly leased watch turn with StateReport::Set and an Expire send. It checks that the pending timer matches armed and has the watch as sender, then runs the real actor and verifies the timer’s FailureCause::Refused, self-target reason, unchanged state/version and no subsequent timer or alarm. This exercises the guard after eligibility. The existing forged/inactive no-op test is unchanged.

Notes: Scope is limited to two virtual-time tests and the handoff update; its count of 27 tests is accurate. The reported guard-removal mutation is consistent with the new assertions. No implementation or protected-document edits occur in this round.

Not checked: Reviewed 2351f94..c013ab2 and necessary design, architecture and API/executor context only. Tests, mutation, formatting, clippy, Docker and gate were not run independently. Builder reports clean formatting/clippy, 516 actor-crate tests and 11 docs tests; full workspace was not rerun and no gate evidence exists for this head. Release checks remain outstanding under the normal gate workflow.
