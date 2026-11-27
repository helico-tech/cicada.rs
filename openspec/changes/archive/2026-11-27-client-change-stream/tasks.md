# Tasks

Normal lane, version 0.26.0 (unreleased). Its crate is `cicada-client` only, so it can build next to an inspector
change. Follow `design.md`; the spec is `specs/change-feed-http/spec.md`; the design reviews are
`.reviews/client-change-stream.design.*`.

- [x] 1. **ADR 0077.** `docs/adr/0077-a-client-for-the-change-stream.md` (partly superseding ADR 0035's "no client
  streaming API"), its row in `docs/adr/README.md` and ADR 0035's status line (decision 1). Verify: `cargo test -p
  cicada-examples --test docs`.
- [x] 2. **Opening the stream.** `ActorClient::changes`: the client's own HTTP client, the per-request timeout of 24 h
  and 1 min, the 10 s wait for headers (`ClientError::Body`), `ClientError::Api` for a status other than 200,
  `ClientError::Body` for another content type; `ClientError`'s rustdoc naming the new `Body` cases (decision 2).
  Verify: the opening tests of the design against an in-process server, and no secret in any error's text.
- [x] 3. **The parser and the stream.** Reading with `Response::chunk()`, the event-stream rules of decision 4, the
  64 KiB line limit, `Idle`, `Lifetime` on the stream's own timer, `Transport`, `Malformed`, a clean end as `None`;
  `Change` and `ChangeStreamError` `#[non_exhaustive]`; `futures-core` as a dependency (decisions 3–5). Verify: the
  crafted-byte tests on virtual time, the real-time test with a short total timeout, and the in-process server tests.
- [x] 4. **Changelog and docs handoff.** The 0.26.0 section names `ActorClient::changes` under Added; `docs-handoff.md`
  carries the design's Docs section, including the line of `docs/architecture.md` to replace. No edit to `README.md`,
  `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc`.
- [x] 5. **Finishing checks.** `cargo fmt --all --check` (lines ≤120, comments included), `cargo clippy --workspace
  --all-targets`, `cargo test --workspace`, `cargo deny check advisories`, `openspec validate client-change-stream
  --strict`, commit. The change loop runs the gate at landing.
