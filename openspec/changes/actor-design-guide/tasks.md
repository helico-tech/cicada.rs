# Tasks

- [x] 1. Session actor and producer protocol. Add `examples/src/websocket_session.rs` with the session ref/state,
  messages and handler, plus a small producer with one pending request/result. Implement the design's frame/byte
  limits, exact sequence range, Accepted/Rejected replies, cumulative acks and unique stream-life key convention.
  Separate public submission errors for NUL from actor-level oversized-body refusal; preserve normal rollback if
  the reply actor disappeared. Add paused-time TestActors cases for acceptance, rejection precedence, UTF-8/count/
  sequence boundaries, unchanged state on rejection, duplicate/backwards/wrong-stream/future acks, capacity recovery
  and reply rollback. Verify `cargo test -p cicada-examples --test websocket_session` with the relevant actor cases.

- [x] 2. Host bridge and runnable websocket demo. Add the public-API state-following bridge, axum ws adapter and
  `examples/examples/websocket_session.rs`, with example-only dependencies. Subscribe before reading; reread exact
  sessions on hints, resync and a one-second timer. Bound I/O, buffers and task lifetimes as designed; read acks
  concurrently with writes and join connection work on error/shutdown. Demo uses loopback, precreated fresh stream
  and producer, and sample output. Include a minimal browser client implementing persistent cursor, duplicate ack,
  gap recovery and local connection generation. Verify `cargo check -p cicada-examples --examples`, protocol/JSON
  unit cases and one TestActors/paused-time real websocket handshake/exchange in the example tests; no real sleeps.

- [x] 3. Recovery tests. Exercise the same bridge loop through a bounded deterministic transport: disconnect before
  ack, unknown/lost ack submission, reconnect through a second ActorSystem sharing the store, overlapping sockets,
  resync/missed hints and periodic reconciliation, stalled writes/store operations, session deletion, errors/feed
  end and shutdown. Check retained output and stopped/joined connection work, not just returned status. Use virtual
  time and TestActors; add no production fault hooks. Verify `cargo test -p cicada-examples --test websocket_session`
  and relevant protocol/client checks. Identify the tests proving acceptance/rollback atomicity for G2's handoff.

- [ ] 4. Guide, routes and doctest wiring. Prepare the docs handoff for `docs/guide/design.md` with the nine headings,
  principle-then-Cicada method, checked examples, diagrams and protocol limits in the design. Request docs-owner
  navigation in root README Documentation, docs/README Solve a task, docs/agents reading map and core Design an actor/
  Examples, plus the G2 test references. Add DesignGuideDoctests to examples/build.rs and src/lib.rs, export the
  example and route skills/cicada/SKILL.md to all new headings and the example; extend REQUIRED_ROUTES. The missing
  guide/routes are blocking: integrate the owner's exact prepared files before landing, never edit protected prose
  or disable checks. Verify `cargo test -p cicada-examples --doc`, `--test docs` and `--test skill` on that result.

- [ ] 5. ADR, release and complete handoff. Reconcile ADR 0050/index and finish docs-handoff.md with final examples,
  tests, client obligations, submission-vs-application refusals, limits and docs-owner integration evidence. Apply
  the next patch release through the normal workspace/lock/skill/plugin/OpenAPI version steps and changelog. No new
  library API, core wire format, migration or guarantee; preserve existing doc anchors. Verify the example tests
  and docs/doctest/skill checks after any version or final integration changes. Do not claim browser execution or
  deployment evidence that was not obtained.

- [ ] 6. Finishing. Run `openspec validate actor-design-guide --strict`. Commit the final implementation tree and
  run `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Its full checks include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill checks and Docker E2E. Keep blocking owner prose integrated and record remaining
  limitations. The builder does not push, tag, release or archive; publication remains with the build loop.
