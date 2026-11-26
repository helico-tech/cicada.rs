# Tasks

Normal lane, version 0.26.0 (unreleased; it already breaks `WorkerReady`). Builds on `tray-menu-bar` (landed). Follow
`design.md`; the specs are `specs/remote-workers/spec.md` and `specs/menu-bar-tray/spec.md`. No ADR.

- [x] 1. **`Worker::end_grace()`.** A token set only while `Stopping`, under the lifecycle lock; `stop_with_grace`
  races the polling-loop wait and the grace wait against it; a plain call; rustdoc next to `stop_with_grace`
  (decision 1). Verify: the core tests of the design on virtual time; `cargo test -p cicada --lib`.
- [x] 2. **`StopNow` in `WorkerReady`.** The handle (a `CancellationToken`), `WorkerReady` without `PartialEq`/`Eq`,
  the worker over HTTP applying it once its stop has set `Stopping`, the `node.rs` test comparing fields (decision 2).
  Verify: the app tests of the design; `cargo test -p cicada-app`.
- [x] 3. **`Tray::stop_now()` and the menu row.** `until` only moves earlier and `stop_now` never calls
  `Connection::stopping`; the menu model's Stop Now row while stopping a ready worker; the AppKit view renders it and
  calls `stop_now` (decisions 3 and 4). Verify: the tray and menu tests of the design; `cargo test -p cicada-tray`;
  `cargo clippy -p cicada-tray --all-targets`.
- [ ] 4. **Changelog and docs handoff.** The 0.26.0 section names `Worker::end_grace`, `StopNow` and `Tray::stop_now`
  under Added, and the `WorkerReady` changes under its BREAKING line; `docs-handoff.md` carries the design's Docs
  section. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test
  docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate worker-stop-now --strict`, commit. The change loop runs the gate at
  landing.
