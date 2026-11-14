---
id: flow-reply-waits.code.1
kind: code
change: flow-reply-waits
round: 1
base: 8605589a613e436c2fd318325cf784d83a834fbc
head: 3fd6aea05819569f2e9517c82362a447954b973b
requested: 2026-11-14T14:34:35+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-reply-waits.code.1 <file>`.

## Note from the requester

Version: 0.18.1

Owner rule for this build (decision 24a): README and hand-written docs belong to the docs session; user-facing prose is in openspec/changes/flow-reply-waits/docs-handoff.md. Review the handoff instead of guide edits.

Builder's report:

Tasks (all done):
1. Request<T>, ReplyTo<T>, ReplyTo::send and Step::request, with the named unit tests.
2. The runner handles Reply without running flow code. Actor tests cover every ignore and fail path, sender checks, correlation and the rejected child create; store_failure.rs covers a request whose registry read fails.
3. Step::wait closes, parks or returns a saved reply. Unit and actor tests cover pokes, pause, lost continuations and recovery after a decode failure.
4. New fixture tests/contracts/test.job.json pins the route's schema; example 17 (examples/src/flow_approval.rs, with tests) added and routed in SKILL.md.
5. Changelog, version, OpenAPI version, skill version, a short "In Rust" paragraph in ADR 0044 and its index row. Guide, architecture, operations and upgrading prose in docs-handoff.md.
6. Finishing checks.

Checks: fmt --check and clippy -D warnings pass; cargo test --workspace --no-fail-fast with PostgreSQL 2786 passed, 0 failed; openspec validate --strict valid. On a first workspace run one core PostgreSQL test (a_cancelled_close_leaves_its_resources_… : "2 before, 3 after") failed and then passed alone 3 times and on the full rerun; this change does not touch the core (handled separately as a flaky test on main). Not run: Docker E2E; the remote-worker sender scenario (the core already sends as the leased actor).

Departures / open:
- Three doc comments that are on the wire (Reply, reply, Wait) were corrected; hashes leave descriptions out so they are unchanged, but the text of the test.sign-up fixture and the flows.onboarding record in contract-texts.json changed. The builder prefers keeping the corrected text (the old text says "refused for good"); the owner is asked, assume it stays.
- A failed send's error now says `request` instead of `send` when the entry is a request.

Example (examples/src/flow_approval.rs):
  let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
  let request = step.request("ask-approval", Target::create(&approvals, &empty), ask).await?;
  let decision = step.wait("decision", &request).await?;
  // in the approvals actor:
  decided.reply.send(turn, &Decision { approved, by }).await?;

New concepts: reply route ReplyTo<T> (run address + request message id carried in the target's message; the target replies from its own turn; only the first reply from the request's target counts); parked run (a wait with no reply saves an open wait and arms no Continue; the reply's turn closes it and arms the next Continue); early reply (kept; the wait closes at once and the value reaches flow code one execution later).
Breaking: none in the Rust API or contract hashes. A 0.18.0 node refuses Reply for good, so every host and worker of a flow type must be upgraded before flow code requests.
