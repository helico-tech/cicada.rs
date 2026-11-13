## Implementation

- [ ] 1. Reconcile the landed predecessor APIs, then add GET /changes on both routers using Params, a prefix-only
  query, axum Sse/Event JSON framing, fixed 15-second keep-alives and X-Accel-Buffering: no. Keep one core receiver,
  no preflight and no extra event queue. Add virtual-time tests in cicada-axum for both routers, first resync/frames,
  canonical encoding, prefix/duplicate/unknown query rules, ignored Last-Event-ID, headers, lag/backpressure, dropped
  bodies, close and worker stop. Test the framing adapter on a pending stream for pre-Up keep-alives without a private
  backend dependency. Add a protocol test for serialized resync/list/get with hints during paging, newer resync,
  bounded pending addresses and retry after failed reads. Run targeted axum tests.
- [ ] 2. Add the private EventStream OperationOutput marker with text/event-stream, string schema, frame examples
  and lifecycle/no-replay description, operationId streamActorChanges and the existing default problem response.
  Extend full/read-only OpenAPI tests for the route, prefix-only query and successful media type. Regenerate and
  commit docs/openapi.json with CICADA_UPDATE_OPENAPI=true; verify the document drift test and unchanged old routes.
- [ ] 3. Add regression coverage using the real /changes route for cicada-app's current stop/drain/cutoff sequence,
  asserting node stop precedes cutoff, the stream cannot block beyond the drain, and the serve lifecycle closes its
  store afterward. Retain production DRAIN=5s and use the existing shortened-drain harness/virtual-time controls in
  tests. Also cover the standalone guide's stop-then-close shutdown with an open stream. Run affected app tests;
  no new shutdown API or production lifecycle change belongs here.
- [ ] 4. Update the axum crate route table, architecture's HTTP/closed-store sections, HTTP guide and operations.
  Include example frames, readiness versus comments, store/worker/app lifetimes, auth/CORS, proxy/timeouts and bounded
  consumer recovery. Write a guide-only JS example with one worker, at most 1000 dirty addresses, generation-aware
  resync, safe fetch-failure retry/disposal and correct path/query encoding. State that Rust protocol tests ground it,
  not JS execution. Fix the runnable standalone axum example to close Store after node.stop inside its shutdown
  future before awaiting graceful drain. Document the unchanged app five-second drain, no Rust client SSE method,
  predecessor coverage limits and no replay. Update changelog/version; run `cargo test -p cicada-examples --doc`,
  `cargo test -p cicada-examples --test docs` and affected crate doctests. Keep actor contracts and CLI unchanged.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate actor-change-sse --strict`; include PostgreSQL where configured and record unavailable optional
  checks. Regenerate version-bearing references if existing drift checks require it; no browser/proxy execution is
  claimed by passing the Rust tests.
