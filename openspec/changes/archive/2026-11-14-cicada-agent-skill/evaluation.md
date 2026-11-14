# Evaluation of the consumer skill

A behavioural check of `skills/cicada`, run once while building the change (task 3). It shows how the skill guided
one independent agent on three requests. It does not prove the skill right for every task.

## Setup

- Two local clones of this branch: one at its head, tagged `v0.17.2` in the clone only, and one at `v0.16.17`.
- The skill packaged from the clone's tag with `scripts/package-skill.py`, unpacked into a staging folder, and copied
  to `.claude/skills/cicada` in three new consumer projects outside the checkout. Each depends on a clone by path,
  with a lockfile made offline.
- An independent agent (Opus 5.5, a general-purpose agent with no view of this repository) got the request, the
  installed skill and the rules: offline, change only the project, no `cargo update` or `cargo fetch`. It was not told
  the expected decisions of `examples/tests/skill/evals.json`.

## Results

| Case | Helper | Decisions and artifacts | Against `expect` / `avoid` |
| --- | --- | --- | --- |
| `duplicate-payment` (payments-service, 0.17.2) | `ok`, `equal`, `same` | Found that the gateway key was built from `turn.current_time()`. Replaced it with `turn.message().id`, keeping it over the order id because one order may have several charges. Added a test that cuts the first turn off after the charge and lets the lease run out: one charge. The test fails with the old code (3000 charged instead of 1500). | Met. Says handlers run at least once and uses a stable key. No claim of exactly-once. |
| `flow-cancellation` (flows-service, 0.17.2) | `ok`, `equal`, `same` | Wrote the flow with `step.run` keys and `step.sleep`. Said the flows of this version have no cancel of their own. Compared deleting the run, cancelling the armed `Continue`, pausing, and a flag checked by a step, and chose deleting the run, warning that a call already in flight is not undone. Three tests pass, one of them the flow's contract snapshot. | Met. No API was invented. |
| `offline-old-source` (audit-service, 0.16.17) | `ok`, `older`, `different` | Said once that the project is older than the skill, then read the 0.16.17 checkout and used its `DeadLetter.payload` as plain JSON, without the `LetterPayload` of 0.17. Added `cicada-actors` from the same old source; nothing was downloaded. One test passes. | Met. No fetch or upgrade. One route (`examples/src/dead_letters_upgrade.rs`) does not exist in 0.16.17; the agent noted it and went on. |

## Findings, and what changed

- **Stopping a flow run.** The skill's line on cooperative cancellation could be read as a business cancel, and the
  flows row did not point to control effects. Fixed: the line now tells the two apart, and the flows row routes to
  `docs/guide/core.md#control-effects`.
- **Dead letters need watches.** Fixed: the ready-made actors row routes to `docs/guide/core.md#watches`.
- **Older sources lack some routes.** Fixed: section 2 says that routes are written for the skill's version.
- **Adding a crate to an existing integration.** Fixed: section 5 says it comes from the same source and version, and
  that the helper reports `unavailable` until the lock is updated.
- **Verify asked for snapshots of every type.** Fixed: snapshots and samples are for types the task adds or changes.
- **Not fixed (a guide gap, outside this change):** the core guide has no recipe for testing "the outside call
  happened, then the node died". `claim_and_abandon` never runs the handler, so the agent used `tokio::time::timeout`
  and the lease, which no guide describes. That recipe belongs in the core guide's testing section, in a later change.

## Limits

One agent and one run per case, three of the nine cases. The cases `new-sqlite-integration`, `moved-heading`,
`mixed-fork`, `http-auth`, `unshipped-feature` and `contract-change-deploy` were not run; the resolver's handling of
mixed, moved and missing sources is covered by its tests, not by an agent run. The judgement is the builder's.
