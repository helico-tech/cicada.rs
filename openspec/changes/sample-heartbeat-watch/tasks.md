# Tasks

- [x] 1. Catalog and contracts. Add HeartbeatWatchActor to the shared sample catalog, without a singleton, Beat or
  startup hook. Regenerate the sample contract snapshot and text digest with the normal update variable. Review the
  new snapshot's parsed JSON against docs/reference/contracts/cicada.heartbeat-watch.json and verify that reference
  and every previous sample entry are unchanged. Run the sample contract_snapshots test and existing wire checks.

- [ ] 2. Focused catalog and transport checks. Cover all, exact-name and none selection for both sample definitions,
  and no automatic heartbeat-watch instance. Extend the predecessor's in-process App/HTTP harness with an explicit
  Counter target and one-hour watch: state plus pending-message reads show waiting/no timer before Beat, then alive
  with a matching armed Expire after commit. Exercise direct-store sample hosting and HTTP executor; do not wait for
  a real deadline. Run targeted sample tests and the existing virtual-time heartbeat_watch tests for waiting until
  first Beat and one alarm after silence. Add no duplicate actor algorithm tests or new Docker class.

- [ ] 3. Handoff and release preparation. Write docs-handoff.md as an addition to sample-worker-mode's already-landed
  App Sample service block: availability, explicit create/Beat and the existing Heartbeat watch guide link. Verify
  the docs session's prose has landed before landing; builders do not edit handwritten docs. Check docs/doctests and
  both skills' existing routes. Prepare the normal patch release pins and release handoff, recording that the image
  inherits the shared catalog and that no NodeDownMonitor wiring or native worker heartbeat was added.

- [ ] 4. Finishing. Run `openspec validate sample-heartbeat-watch --strict`. Commit implementation and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Its full checks include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill checks and existing Docker E2E. Record actual verification and limits. The builder
  does not push, tag, release or archive; publication remains with the build loop.
