---
id: flow-wait-timeout.code.1
kind: code
change: flow-wait-timeout
round: 1
base: c55a445e7aa0c82f724a0d4aa68cfb36c877ea4a
head: 52db637b362205f4eb5bf551d61e92af04dcadac
requested: 2026-11-14T15:10:44+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-wait-timeout.code.1 <file>`.

## Note from the requester

Version: 0.18.2

Owner rule for this build (decision 24a): README and hand-written docs belong to the docs session; user-facing prose is in openspec/changes/flow-wait-timeout/docs-handoff.md. Review the handoff instead of guide edits.

Builder's report:

Tasks (all done):
1. Waited<T> and Step::wait_timeout; wait and wait_timeout share one core; sleep's wake-time helper now takes a kind name.
2. Runner behaviour.
3. Race and recovery tests.
4. Example 17 now waits a week (DECISION_TIMEOUT); Orders gains an `expire` method.
5. ADR 0044 paragraph and index row, changelog, version, OpenAPI, skill version. Guide, patterns, architecture, operations, upgrading and core-guide prose in docs-handoff.md (its guide snippet was compiled once).
6. Finishing checks.

Checks: fmt --check and clippy -D warnings clean; cargo test --workspace with PostgreSQL 2814 passed, 0 failed; openspec validate --strict valid. Not run: per-feature cargo check builds, Docker E2E (the release runs both).

Departures / open:
- Contract hashes unchanged (fixture check confirms), but the wait entry's out-of-date doc comment changed its description text in tests/contracts/test.sign-up.json and sample-service/tests/contract-texts.json (the owner accepted the same kind of description correction for flow-reply-waits).
- No way to inject a failing commit for the "failed decision" test; instead the flow panics after a caught timeout decision, so nothing commits, and then a reply wins. Please judge whether this proves the scenario.

Example (examples/src/flow_approval.rs):
  match step.wait_timeout("decision", &request, DECISION_TIMEOUT).await? {
      Waited::Received(Decision { approved: true, .. }) => { /* place-order */ }
      Waited::Received(Decision { approved: false, by }) => { /* notify-rejected */ }
      Waited::TimedOut => { step.run("notify-expired", |key| async move { Ok(orders.expire(item, &key).await?) }).await?; }
  }

New concepts: Waited<T> (a timeout is a value, not an error, so retry policy never sees it); deadline (counts from the turn that creates the wait, saved; replay keeps it even if the code changes the timeout); race rule (whichever of reply and deadline the run's mailbox handles first wins, even a reply handled after the deadline; once committed the outcome never changes; a timeout does not stop the target).
Breaking (builder's list): Orders in example 17 has a new required method (examples crate only, not a library API); 0.18.1 hosts permanently refuse a reply to a wait with a deadline, so every host and worker of a flow type must be upgraded before code uses one; a running run cannot switch between wait and wait_timeout (its replay fails for good).
