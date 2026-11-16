Verdict: not yet

Blocking finding:

1. The promised NUL refusal cannot reach the session actor. In `openspec/changes/actor-design-guide/specs/actor-design-guidance/spec.md`, “Refuse without losing accepted output” requires `Rejected { reason: InvalidBody }` when the body contains NUL; design §6/§11 also group body validation with application replies/tests. But `ActorSystem::send_message_checked` (`crates/cicada/src/runtime/system.rs:1268`) rejects NUL before enqueue, and `Turn::buffer`/`check_send` (`crates/cicada/src/turn.rs:691`, `:754`) reject it before buffering. Thus no session turn can produce that reply through the public API. This contradicts the current architecture’s HTTP-contract validation and the no-core-change scope. Separate NUL into a submission-error scenario (`OpError::Invalid` or `TurnError::Invalid`, no application reply); retain actor-level InvalidBody for oversized, otherwise storable strings. Make the boundary tests assert those distinct outcomes and update the handoff.

Note:

- Add root `README.md` to the docs navigation handoff: the existing `openspec/specs/documentation/spec.md` requirement “There is one guide per layer” says the README names every guide, while design §12 currently names only `docs/README.md`, `docs/agents.md` and core.md.

Owner-only questions: none.

Checked: change artifacts, relevant architecture/ADRs, existing documentation specification, predecessor/successor proposals, send/turn validation, change-hint APIs, example/doctest/skill wiring. `openspec validate actor-design-guide --strict` passed.

Not checked: implementation (not yet present), browser behavior, Rust tests, PostgreSQL/Docker gate or upstream websocket API claims.
