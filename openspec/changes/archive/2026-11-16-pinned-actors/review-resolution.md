# Design review resolution

Opus 5.5 returned ready for tasks for pinned-actors.design.1 on 2026-11-16. No blockers or owner-only questions.
The owner delegated design/task approval and authorized this batch beyond the usual two-ready limit.

- Add a partial placement/type/due index in migration 8, retaining the existing due index. Planner choice is not
  guaranteed; stranded foreign pins are excluded before claim limits and from this owner's next-due result.
- Require new fixed lease cases for valid pinned creation and invalid pin conversion, plus coverage declaration;
  retain every historical case's exact bytes. ADR 0047 remains the fixture contract.
- Allow old HTTP workers to process matching pins: upgraded servers enforce claim eligibility. Older workers cannot
  express new pin creation options; all old direct-store runtimes still must be drained before migration.
- Explicitly exclude pin options in supervisor/flow/event protocols here and in the process example, which creates
  its pinned instance directly. A ready-made supervisor cannot recreate that pin with its current protocol.
- Clarify HTTP callers must GET to verify existing placement, and name invalid-pin errors at every public boundary.

The five implementation tasks remain bounded after the process-example split. These are narrow clarifications;
no second design review is needed. Strict OpenSpec and docs checks validate the artifacts, not implementation behavior.
