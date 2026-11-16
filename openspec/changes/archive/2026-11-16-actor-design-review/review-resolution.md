# Design review resolution

Opus 5.5 reviewed actor-design-review.design.1 on 2026-11-16 with verdict not yet. The owner delegated decisions
and approval to the design session. Both blockers and all five notes are resolved without changing the workflow.

- Add a MODIFIED delta for the predecessor's portable actor-design requirement, replacing its temporary exclusion
  of review with both modes and updating Choose the appropriate skill, including cicada's entry point. The ordered
  predecessor must be archived first; its currently unimplemented files are not changed by this design.
- Explicitly allow read-only inspection and the bundled resolver with locked/offline Cargo metadata. The execution
  restriction covers consumer-project builds, tests and scripts, not dependency discovery.
- State three focused questions per turn. Give findings monotonically allocated R-001-style ids, preserving ids
  for the same cause/location, never renumbering or recycling retired ids.
- Resume when report Scope/Progress identify the same system and review scope, even at changed revisions; treat
  unrelated or uncertain identity as a collision requiring clarification or a different destination.
- Name skill.rs as the checker of review-case shape/coverage and fixture existence/safe relative paths; fixtures
  are test-only. Route state/bounds to the guide's existing Choose actors and identities section.

These are narrow specification and testability corrections. No second review is needed. Strict validation and
documentation checks accompany the ready commit; actual method behavior and packaging remain implementation checks.
