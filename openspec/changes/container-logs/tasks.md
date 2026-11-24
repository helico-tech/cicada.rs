# Tasks

Normal lane, additive: 0.20.9, with the rest of the container line, released when the line, through
`container-limits-and-kinds`, has landed (check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Builds on `container-consistency`, which must have landed first. Follow `design.md`; the specs are
`specs/container-runtime/spec.md` and `specs/container-docker/spec.md`. Run the crate's tests with every feature:
`cargo test -p cicada-containers --all-features`.

- [x] 1. **Types, cursor and pager.** In `crates/cicada-containers/src/`: `LogPage`, `LogLine`, `LogStream`
  (`non_exhaustive`, with constructors), `LogCursor` with its private form `S:n`, `Display`, `FromStr` (`InvalidSpec`
  for a bad string) and serde and `JsonSchema` as a string (decision 2), `LOG_PAGE_MAX` (1000), and the private pager
  of decision 6: it drops lines stamped before `S`, skips `n`, takes up to `max_lines`, and gives `next` with the
  count that includes the skipped lines (decision 1). Export the public types from `lib.rs`. Verify: unit tests of the
  pager (lines sharing a second, out of time order within a second, a page cut in a second, an empty read keeping the
  cursor, a repeated cursor, and lines A, B, C of decision 1: B dropped, C not again) and of the cursor (round trip
  through `Display`/`FromStr` and JSON, bad strings); `cargo test -p cicada-containers --all-features`.
- [x] 2. **The trait and the fake.** `ContainerRuntime::logs_since` with a default that fails with `Other` (decision
  5); its rustdoc names the bound, what a missing and a created container give, that it is called outside actor
  turns, and the limits of the design (a late-stamped line missed, gentle polling, rotation, lines over 16 KiB in
  parts, bollard's 32-bit `since` after 2038). The fake (decision 7): one timed line store, `push_log`, `set_logs` as
  `Out` lines at the Unix epoch, `logs` from the same store, `logs_since` through the pager, `Call::LogsSince` and
  `Method::LogsSince`. From Factory's review, the rustdoc also says: a page with fewer than `max_lines` lines has read everything there is
  now (still true after the pager drops early lines), and which error kind a log driver that cannot be read (for example
  `none`) gives — not `DaemonUnavailable`, so a follower stops retrying. Verify: a unit test that a runtime without `logs_since` compiles and fails with `Other`; the
  fake's tests for paging timed lines, `set_logs` read by both calls, `Conflict` for a missing container, an empty
  page for a created one, `InvalidSpec` for `max_lines` 0 and 1001, and a scripted failure;
  `cargo test -p cicada-containers --all-features`.
- [x] 3. **The Docker backend.** `DockerRuntime::logs_since` in `src/docker/mod.rs`, with the pure parts in
  `src/docker/map.rs`: check `max_lines` first; `Conflict` for a missing container and for a 404 during the read;
  read with `timestamps=true`, `follow=false` and `since=S`; parse each frame into a `LogLine` (time by
  `cicada::time::parse_instant`, stream from the frame, lossy text without the newline; `Other` for a frame without
  a time); feed the pager and drop the stream once the page is full. A cursor whose second does not fit an `i32`
  fails with `Other`. Verify: unit tests of the frame parser (out and err, nanoseconds, a missing newline, a byte
  that is not UTF-8, no time) and of the `i32` check; `cargo test -p cicada-containers --all-features`.
- [ ] 4. **E2E, changelog and docs handoff.** In `e2e/tests/suite/container_docker.rs`, the cases of the design's
  Tests section: pages of 7 through 50 lines on both streams in one second; a follower polling a container that
  prints across several seconds; the same cursor twice; a missing container (`Conflict`), `max_lines` 0 and 1001
  (`InvalidSpec`) and a created container (empty page); a byte that is not UTF-8. The 0.20.9 section of
  `CHANGELOG.md` names `logs_since`, `LogCursor`, `LogPage`, `LogLine`, `LogStream`, `LOG_PAGE_MAX` and the fake's
  `push_log` under Added. `docs-handoff.md` carries the call, the cursor, the limits (gentle polling, lines over
  16 KiB in parts, 2038) and the follower loop for the runtime section of the container guide; nothing blocks. No
  edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `CICADA_E2E=1 cargo test -p cicada-e2e --
  --nocapture` (or the gate below); `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-logs --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
