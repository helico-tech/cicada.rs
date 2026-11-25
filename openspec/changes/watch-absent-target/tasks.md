# Tasks

Normal lane, **BREAKING** (a field on the public `WatchChange` and `WatchChangeView`): the unreleased batch becomes
0.22.0. If the `release:` line of `python3 scripts/status.py` names an unreleased patch version (0.21.2 from
`docker-context`), rename that section of `CHANGELOG.md` to `## 0.22.0 — unreleased` and rewrite its summary for the
whole batch; set 0.22.0 wherever the version lives (builder.md, Version). Follow `design.md` and ADR 0063
(`docs/adr/0063-a-watch-of-an-actor-that-must-exist.md`, committed with this change); the specs are
`specs/watches/spec.md` and `specs/remote-turn-http/spec.md`.

- [x] 1. **The turn call and the model.** `Turn::watch_existing(target)` in `crates/cicada/src/turn.rs` (one actor only;
  a type prefix or `*` is `TurnError::Invalid`; the family check of `watch`), `WatchChange::existing`, and the
  rustdoc of both methods (decisions 1 to 3; the `Stopped` names the absent actor; `watch` points to
  `watch_existing`). The in-memory store's commit checks the target in the order of the calls, writes the `Stopped`
  as a notification of the commit's event and leaves the pair without a watch. Conformance tests for every case of
  the design's Tests section. Verify: `cargo test -p cicada --features testing`.
- [x] 2. **SQLite and PostgreSQL.** The same commit on both backends (decisions 4 and 5): on PostgreSQL the target joins
  the ordered `FOR UPDATE` lock set and "present" means the lock found its row, before the rows of the turn's
  creates; on SQLite inside the commit's write transaction. A PostgreSQL test races a delete against the commit for
  many rounds and finds exactly one `Stopped`. Verify: the conformance suite on SQLite, and with
  `CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada` on PostgreSQL.
- [ ] 3. **The wire.** `WatchChangeView` gets `existing` (left out when false); the view's conversion to `WatchChange`
  refuses `existing` on a type target or with `set: false` as an invalid report (decision 6); `cicada-client` sends
  it. Regenerate `docs/openapi.json` (`CICADA_UPDATE_OPENAPI=true`). `fixtures/leases.json` gains an accepted body with
  `existing` and a refused one, checked by `crates/cicada/tests/fixtures.rs`. A remote-worker test runs a turn with
  `watch_existing` over HTTP and finds the same `Stopped` as a local commit. Verify:
  `cargo test -p cicada --test fixtures`, `cargo test -p cicada-client`, `cargo test -p cicada-axum`.
- [ ] 4. **The pool case, changelog and docs handoff.** A runtime test on virtual time with `TestActors`: a run deleted
  between the message that gives it a place and the pool's turn frees the place through `watch_existing`. The
  changelog names `Turn::watch_existing` under Added and the `WatchChange` field under a **BREAKING** line, with the
  upgrade order (servers before workers). `docs-handoff.md` carries what the design's Docs section lists; nothing
  blocks unless a check says so. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --test docs`, `--doc` and `--test skill`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace` with PostgreSQL, `openspec validate watch-absent-target --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches the HTTP crates).
