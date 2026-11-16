# Tasks

- [ ] 1. Actual executor identity. Make SlowActor, CounterActor, RecorderActor and EffectLog record Turn::node_id
  from pinned-actors. Remove captured identity fields and sample host node_config/startup identity checks. Preserve
  public constructor signatures, document ignored legacy identity arguments, and use private construction without
  manufacturing ids. Verify paused TestActors tests with conflicting legacy/runtime ids, existing state/message
  samples and unchanged contract snapshots/texts. Replace the node-id flag refusal check with correct precedence.

- [ ] 2. Separate executable App. Add Sample::executor and cicada-sample-executor, sharing catalog, routes/auth and
  main/telemetry setup while omitting on_start hooks. Retain Sample::new bootstrap and set package default-run to
  cicada-sample-service. Keep App's full command/lifecycle policy. Verify sample construction, help/version/default
  binary, old-variable rejection, both definitions' catalog parity and original bootstrap tests.

- [ ] 3. HTTP acceptance. Extend sample-service tests with a send-only server and executor: explicit server plus
  CICADA_TOKEN, unusable ambient store, selected contracts, no bootstrap, and a counter run under the flag-selected
  worker id. Check missing/wrong credentials, explicit store conflict, original hook rejection and graceful stop.
  Use the bounded socket harness; keep actor timing/retry assertions on virtual time. Run targeted sample tests
  and existing App HTTP-worker checks; no changes to App resolution, contracts or worker presence.

- [ ] 4. Container delivery. Ship both binaries in the existing Dockerfile, preserving default server entrypoint.
  Add the isolated authenticated server/executor E2E class with health check disabled, no supplied database volume,
  inaccessible ambient store, CICADA_TOKEN and worker identity/counter assertions. Verify SIGTERM exit 0, existing
  server bootstrap, and failure cleanup including anonymous volumes/network. Run the targeted class through the
  suite's normal Docker harness and register it for the committed full gate.

- [ ] 5. Handoff and release preparation. Write docs-handoff.md naming the App sample block, operations deployment
  guidance and e2e/README.md; include bootstrap duties, constructor compatibility, tokens, entrypoint/health check,
  store precedence and the now-valid --node-id override. Verify the docs session's prose has landed before landing;
  builders do not edit handwritten docs. Check docs, doctests and both skills' routes. Prepare normal patch release
  pins and release handoff. HeartbeatWatch hosting/snapshots belong to sample-heartbeat-watch, not this task.

- [ ] 6. Finishing. Run `openspec validate sample-worker-mode --strict`. Commit implementation and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Its full checks include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill checks and Docker E2E. Record actual verification and remaining limits. The
  builder does not push, tag, release or archive; publication remains with the build loop.
