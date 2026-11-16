# Tasks

- [x] 1. Review mode and evidence method. Extend skills/actor-design/SKILL.md with mode selection, four review
  phases, source/scope recording, claim categories, reconstruction diagrams and grounded findings/tradeoffs/unknowns.
  Keep the existing design interview unchanged and update cicada's entry point by name. Reuse the read-only resolver;
  do not execute consumer builds/tests/scripts merely for review. Map state/bounds to Choose actors and identities.
  Verify representative sound, defective and incomplete-evidence conversations against the spec, recording maps,
  findings, failure traces and unverified behavior for independent code review.

- [x] 2. Preserved reports and resumption. Define the report skeleton in SKILL.md, default actors-review.md output,
  same-scope resume versus collision rules, chat-only fallback and preservation of original design/user annotations.
  Implement the method for R-001-style finding ids, stale evidence, dispositions and evidence-backed resolution.
  Add staged resume/collision cases that change source snapshots and retain ids/history. Verify through representative
  saved-report runs, comparing before/after artifacts and confirming no unrelated content or design-mode progress
  was overwritten; no new archive member or generic report engine.

- [x] 3. Structural and behavioral coverage. Extend examples/tests/skill/actor-design-evals.json and add small
  review-fixtures inputs for the design's sound alternative, duplicate effect, stale reply/key reuse, wait cycle,
  unknown guarantees, docs/code conflict, old/mixed source, partial scope, mode selection and report cases. Extend
  skill.rs to check case shape/coverage, safe fixture paths/existence, report instructions and routes, retaining
  design cases. Verify `cargo test -p cicada-examples --test skill`, including existing package checks that updated
  skill bytes ship with unchanged members/provenance. Record representative behavioral outputs and their limits;
  structural tests do not prove model compliance and no model service enters the gate.

- [x] 4. Handoff and release. Write docs-handoff.md for mode selection and output paths in install guidance,
  docs/agents.md and docs/README.md. Preserve guide headings, links and guarantee rows; integrate exact docs-owner
  fixes if protected-prose checks block. Apply the next patch release through workspace/lock, both skill versions,
  plugin, OpenAPI pins and changelog. Keep archive format and member set intact. Verify docs/doctests/skill checks
  after integration and version changes, and report behavioral evidence without claiming unperformed checks.

- [ ] 5. Finishing. Run `openspec validate actor-design-review --strict`. Commit the implementation and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Its full checks include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill checks and Docker E2E. The predecessor's spec must be archived before applying
  this change's MODIFIED requirement. The builder does not push, tag, release or archive; publication stays with
  the build loop.
