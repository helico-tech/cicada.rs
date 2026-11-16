# Tasks

- [x] 1. Run actor and bounded host protocol. Add examples/src/pinned_process.rs with desired/observed state,
  node/host-epoch checks, Start/Stop/Refresh/Observed, 32 retained slots, monotone revisions and absorbing terminal/
  NeedsReconciliation states. Reserve before spawn, retain sticky stop through lost commits, expose bounded refusal
  and never recycle tombstones. Define a deterministic fake process adapter. Verify focused paused-time TestActors
  cases for start/repeat, stop-before-start, races, natural exit, capacity, wrong ids/epochs and stale observations.

- [x] 2. Runtime and delivery recovery tests. Add the bounded observation pump after system construction and
  startup paging/Refresh for the node's existing runs. Use a public TurnSource wrapper to withhold completion and
  force a lost-commit replay against the same host table; assert one spawn. Test lost Stop commit, send uncertainty,
  stalled operations, deletion, old-epoch zero OS actions, late original-host exit after NeedsReconciliation and
  operator deletion after simulated external verification. Verify examples/tests/pinned_process.rs on virtual time.

- [x] 3. Real adapter and demo. Add examples/examples/pinned_process.rs using only its fixed inert child mode,
  host-supplied executable path, bounded control, explicit stable node id/SQLite path and explicit new-run creation.
  Own/reap handles in per-slot tasks; implement the defined cleanup/flush/system-stop/pump/store shutdown order and
  report uncertainty. Add tests/pinned_process_os.rs with bounded subprocess helpers and handshakes on Linux/macOS.
  Verify real normal exit, repeated start and stop/reap, plus `cargo check -p cicada-examples --examples`. Record OS
  coverage and limits; no shell commands, PID adoption, arbitrary process manager or guaranteed abrupt-crash cleanup.

- [ ] 4. Example routes and docs integration. Export the module and update both consumer skills' source/example
  route coverage after their predecessors land. Write docs-handoff.md for design-guide outside-system/websocket
  comparison, core Examples and operations lifecycle/cleanup/reconciliation guidance under existing headings. Ask
  for exact owner-prepared prose where docs/doctest/skill checks require it and integrate before landing. Verify
  `cargo test -p cicada-examples --doc`, `--test docs` and `--test skill`; claim no new core G/S guarantee.

- [ ] 5. ADR and release evidence. Reconcile ADR 0053/index and finish handoff with precise protocol, proving tests,
  platform evidence and host-lifetime limits. Apply the normal next patch version/pins/changelog. Re-run affected
  example/OS and docs/skill checks after final integration. State explicitly that uncertain runs need outside
  verification and may then be deleted; automatic replacement and ready-made supervisor pin options remain absent.

- [ ] 6. Finishing. Run `openspec validate pinned-process-example --strict`. Commit implementation and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Its checks include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill checks and Docker E2E. Record unverified platforms/behaviors. The builder does
  not push, tag, release or archive; publication remains with the build loop.
