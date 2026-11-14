---
id: flow-wait-join.code.1
kind: code
change: flow-wait-join
round: 1
base: 88f064aff5477ce3091472f2b6bbcfe81a13aa7f
head: 3cbc05a5d5f80f15844e59daf22c261a1ee2ba62
requested: 2026-11-14T15:50:42+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-wait-join.code.1 <file>`.

## Note from the requester

Version: 0.18.3

Owner rule for this build (decision 24a): README and hand-written docs belong to the docs session; user-facing prose is in openspec/changes/flow-wait-join/docs-handoff.md (its two code blocks were compiled). Review the handoff instead of guide edits.

Builder's report:

Tasks 1–6 done. The four join helpers and the reply handling share one wait engine with the single waits. Tests cover every spec scenario, including rejected commits injected through a worker's TurnSource. The old test expecting joins to be refused is removed (that behaviour no longer exists).

Checks: fmt --check and clippy -D warnings clean; cargo test --workspace with PostgreSQL 2847 passed, 0 failed; openspec validate --strict valid; the flow contract hash is now pinned in a test and equals the v0.18.0 fixture's hash.

Open point, decided by the build loop (owner away, trusting the loop's picks): two spec scenarios ("Reserved active wait" and "Deadline behavior on an older host") describe what 0.18.2 and 0.18.1 do. This branch cannot run their code; the proof is the existing test `a_reply_that_would_close_a_reserved_wait_is_refused_for_good_and_saves_nothing` at tag v0.18.2. The branch adds a test that single waits saved by 0.18.2 replay through the new one-request join. The build loop considers that sufficient; flag it if you disagree.

Example (examples/src/flow_approval.rs):
  for team in &input.teams {
      let approvals = Approvals { team: team.clone() };
      let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
      requests.push(step.request(&format!("ask-{team}"), Target::create(&approvals, &empty), ask).await?);
  }
  let slots = step.wait_all_timeout("sign-offs", &requests, DECISION_TIMEOUT).await?;

New concepts: join (one wait for up to 100 requests of one reply type; results in list order; on timeout each request gets Received or TimedOut, so replies that came are kept); first wait (the winner is the reply the run accepted first, not the first in the list; the result gives the winner's position); closed losers (when a join closes all its requests close; losing targets keep running, late replies are ignored, cleanup needs a new request).
Breaking: none to API, wire or contract hashes. Upgrade every host and worker of a flow type before its code joins (0.18.2 refuses a reply that would close a join for good). The Orders trait in example 17 gains a `buy` method (examples crate only).
