# Design review resolution

Opus 5.5 returned ready for tasks for sample-heartbeat-watch.design.1 on 2026-11-16, with no blockers or owner-only
questions. The 288-word report was counted before hand-in. The owner delegated design choices, tasks and ready
approval for this batch, including the split requested in the executor review and its ready-queue exception.

- Drop the tautological catalog-versus-contract_of check. Review the new snapshot against the pre-existing
  ready-made reference's parsed JSON, then use the existing snapshot and text-digest checks to pin it. No reference
  or prior sample contract changes are allowed.
- Make this docs handoff explicitly build on the predecessor's landed Sample service prose, avoiding stale edits.
- Specify actor-state and GET /actors/{type}/{key}/messages reads before Beat, and a matching pending Expire after
  Beat commits. The initial absence assertion is before sending Beat, not while a Beat itself could be pending.
- State in both design and spec that the new smoke uses in-process App/HTTP transport. Existing Docker gate checks
  still run, but no new container test or container-specific heartbeat evidence is claimed.
- Remove the unlinked decision number from this proposal; the owner-authorized scope and split remain explicit.

These are narrow verification and documentation clarifications; no second review is needed. Runtime/build tests,
predecessor harness compatibility and full release checks remain implementation work. Design-stage strict validation
and documentation checks do not establish those results.
