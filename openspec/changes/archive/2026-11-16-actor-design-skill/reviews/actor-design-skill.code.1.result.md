Verdict: fix needed

Blocking findings:

1. `scripts/test_gate_release.py:519`: the “Resume format 2 after a driver update” scenario is not exercised. This test changes only skill files; `gate.py` remains identical. The existing runner pin correctly forbids using a changed driver, so keep that protection. Extend the sandbox case: save format-2 digests, update the caller’s driver, assert refusal without publication or digest changes, then resume with the pinned driver and verify both manifests/members and saved digests. Reconcile the scenario wording with that supported recovery path. Checkout drift alone does not cover the requested update boundary.

2. `skills/actor-design/SKILL.md:148` and `:155`: revision and the no-actors ending unconditionally require user confirmation, contradicting the scoped delegation rule at line 121. For example, a user delegates all design choices and the agent decides a transaction suffices; the finish rule still blocks on confirmation. Likewise, an in-scope revision loses its delegated status until confirmation. Apply the existing delegation exception to these branches, preserve reasons/uncertainties, and exercise delegated no-actors completion and an in-scope revision in staged behavioral review.

Notes: The draft/needs-review distinction, prioritization within three questions, and waiting for earlier unresolved steps are reasonable judgment calls. The seven clarifications otherwise fit the design. Packaging, legacy compatibility, version pins, ADR/index and docs ownership/handoff agree with the change. Gate status confirms full and E2E passes for the reviewed head.

Not checked: suites were not rerun; no live plugin discovery or additional model/harness trials. Behavioral assessment used the recorded walkthroughs and staged cases, not retained full transcripts.
