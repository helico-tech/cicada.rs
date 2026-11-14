---
id: flow-step-sends.code.1
kind: code
change: flow-step-sends
round: 1
base: d600969c3fd455afa4e18852124d3ffdb863d87e
head: e0c50cc0a3ef49fbb7a72072d1e420f9e1948304
requested: 2026-11-14T13:52:45+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-step-sends.code.1 <file>`.

## Note from the requester

Version: 0.18.0 (breaking)

Owner rule for this build (decision 24a): README and hand-written docs belong to the docs session; user-facing prose is in openspec/changes/flow-step-sends/docs-handoff.md. Review the handoff instead of guide edits.

Builder's report:

Tasks (all done):
1. The shared flow wire from ADR 0044, its state checks, regenerated flow contracts and snapshots. Reply, Cancel and cancelling runs are refused for good.
2. Target and Step::send, with tests for conversion counts, caught panics at each conversion point, unpolled futures, replay misuse and the 1000-entry limit.
3. The runner commits the create, the message, the send entry and the next Continue together. Tested on virtual time, plus a registry outage on SQLite with the existing fault harness.
4. Progress saved by 0.17 replays and goes on; a 0.18.0 state sample; the event regression with its contract hash pinned unchanged; the onboarding example under a new type name next to a 0.17 registration (examples/tests/flow_onboarding.rs).
5. Changelog, a short "In Rust" paragraph in ADR 0044, OpenAPI version, the skill's cicada-version, doc comments. Guide, architecture, upgrading and guarantee prose in docs-handoff.md.
6. Finishing checks.

Checks: fmt --check and clippy -D warnings pass; cargo test --workspace with PostgreSQL 2754 passed, 0 failed; openspec validate flow-step-sends --strict valid. Not run: Docker E2E.

Departures:
- A store error keeps its typed chain and cause `node`. An Invalid or SendRejected error becomes HandlerError::permanent with a formatted message, not permanent_from; the cause is still `refused`.
- The upgrade example is a test in the existing onboarding example, not a new examples/src module, to avoid editing the skill's routes.

Example (Step::send doc comment):
  let job = Job(name);
  step.send("start", Target::child(&job, &JobState { size: 3 }), JobMsg::Start { size: 3 }).await?;
  step.run("wait-for-it", |_key| async { Ok(()) }).await?;
  step.send("stop", &job, JobMsg::Stop).await?;

New concepts: send step (target create if any, message, saved entry and next Continue commit together; a retry of a later step never sends again; message queued once, not target success; no retry policy on it). Target (&job existing; Target::create independent; Target::child a child of the run, removed when the run is deleted). Reserved wire (replies, waits, cancellation, cleanup journal present in state and contract, refused in this release, enabled later without another contract change).

Breaking: every flow type's contract changes (a 0.18.0 node hosting a flow type won't start against its 0.17 registration: keep old types on pinned 0.17 binaries and host new code under new type names, or drain runs and reset registrations); exhaustive match on FlowMsg, FlowStatus or JournalEntry and a FlowState { .. } literal without ..FlowState::new(input) no longer compile; state written by 0.18.0 always includes "replies": [] and "cleanup": [].
