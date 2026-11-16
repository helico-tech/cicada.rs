# Design review resolution

Opus 5.5 reviewed actor-design-skill.design.1 on 2026-11-16: ready for tasks, no blocking findings or owner-only
questions. The owner delegated decisions and approval to the design session. The five notes are resolved here.

- Retain one change: the owner requested both distribution forms together. Five bounded tasks cover skill,
  structural/evaluation checks, packaging, release verification, and handoff/versioning. No general archive framework.
- Template and Progress take the landed guide's exact nine step names/order; skill.rs checks agreement. Research
  names are input rather than a competing source of truth. The guide's implementation remains with its builder.
- Require a format-2 release-resume case with previously saved digests across a driver update; mismatches fail.
- The coding skill mentions actor-design by name, avoiding a sibling path that its source-route parser would reject.
- Place staged cases at examples/tests/skill/actor-design-evals.json. Live plugin discovery is a manual harness
  check; CI checks structure and coverage only, and unverified installation behavior must be disclosed.

These clarify the reviewed design without changing its method or layout. No second review is needed. Strict
OpenSpec validation and documentation link checks accompany the ready commit; implementation and live harness
behavior remain unverified until built and checked.
