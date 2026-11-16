# Design

The owner delegated the remaining choices and requested Opus 5.5 review. This change composes ADR 0042 after
`node-down-alert`; it also relies on the preceding `pinned-actors` accessor. No core/App semantics or new ADR.

1. **Separate App definition.** Add `cicada-sample-executor` at sample-service/src/bin/cicada-sample-executor.rs.
   `Sample::executor(args, env)` builds the same catalog, configuration, routes and authentication as `Sample::new`,
   but never registers an on_start hook. Share their private construction; keep `Sample::new`'s singleton creation
   and dead-letter watch. A separate definition makes absent bootstrap explicit; no heuristic target parsing or
   silent hook skipping in App. The original binary continues to reject HTTP worker mode under ADR 0042.
2. **Normal App command line.** Use `cicada-sample-executor worker --server URL --types ...`. Keep App's whole command
   surface, including help/version/operators and direct-store worker/serve. Those two store modes use the same
   executor definition without bootstrap, with the same authenticated sample routes when serving. This is an
   executable App definition, not a worker-only CLI wrapper. Set package default-run to cicada-sample-service so
   existing `cargo run -p cicada-sample-service -- serve ...` still works. Share main/telemetry setup in the sample
   crate and call App::run_async for logging, signals, exit codes and graceful stop; do not copy the worker lifecycle.
3. **Existing resolution rules.** App owns --server/--context/--token, CICADA_* variables, type selection, readiness,
   registration, timeouts and diagnostics. Explicit --server overrides the image's ambient CICADA_STORE; an explicit
   --store conflicts, and environment-only HTTP selection still loses to CICADA_STORE. Document explicit --server
   for the image. API_TOKEN protects a served API; it is not the worker's client token. Use CICADA_TOKEN in deployment
   examples and E2E, not a token value in process arguments. Legacy sample variables remain rejected for node commands.
   No database-free build claim:
   the package still includes both SQL features, but the HTTP execution path never opens a database.
4. **Bootstrap boundary.** The executor registers selected contracts but creates no sample singletons or wildcard
   dead-letter watch. A host running the original sample with those types, or an explicit API bootstrap client,
   supplies any actors/watches a workload requires. A send-only server plus the executor does not bootstrap them.
   HTTP workers expose neither an HTTP listener nor /e2e/effects, even though the common App has routes for serve.
   Their in-memory EffectLog is local and not remotely readable; durable E2E actor state proves which worker ran.
5. **Correct identity.** SlowActor, CounterActor, RecorderActor and EffectLog record turn.node_id(), for local and
   HTTP execution. Remove captured node fields but preserve the existing public actor/Catalog constructor signatures:
   their legacy node-id arguments are accepted and ignored, documented explicitly. Use private runtime-based
   construction internally, without manufacturing an id to pass to compatibility constructors. Keep CounterActor's
   shared running-state map. This avoids breaking unknown source consumers of the unpublished sample crate.
   Remove the sample on_start identity check and both definitions' node_config override; App alone resolves node ids
   from flags, then environment, then its random default. The public config::node_id helper may remain for callers,
   but neither sample definition uses it. Check differing constructor/flag/environment ids without changing App.
   Update sample-only callers/doc comments. Existing state, message, schema descriptions and contract bytes stay
   fixed; only the recorded identity and formerly refused flag case change. Test the actual lease owner, not the
   server id or pre-configuration id. Runtime identity is trusted metadata, not proof of a unique physical process.
6. **Image.** Build/copy both binaries to /usr/local/bin in sample-service/Dockerfile. Preserve server ENTRYPOINT,
   user, volume, defaults, stop signal and health check. Worker invocation overrides entrypoint with
   /usr/local/bin/cicada-sample-executor, supplies `worker --server URL`, and disables the image's HTTP health check
   (`--no-healthcheck`, or Compose healthcheck.disable). No new port, readiness listener, image or shell wrapper.
7. **Focused verification.** In sample-service/tests, prove catalog parity, no executor bootstrap on a fresh store,
   unchanged original bootstrap, help/version/default binary and auth/target rules. Paused TestActors tests prove all
   three E2E actors and their effects use the configured turn owner despite differing legacy constructor ids; retain
   the actor timing tests. Replace the differing-node-id rejection test with success.
   Bounded HTTP integration checks run a send-only server and executor, register contracts, create/send/read a
   counter and assert worker identity with --node-id overriding an environment id. An inaccessible ambient store
   must not be touched. Check missing/wrong token failure and original Sample HTTP-hook rejection. Socket/process
   checks use the existing harness; actor deadline/retry assertions stay on virtual time, not wall-clock sleeps.
8. **Container acceptance.** Add a class to the existing Docker E2E suite that starts an authenticated send-only
   sample server and the executor from the same image on an isolated network. Disable worker health checking, give
   it no supplied/shared database volume and an unusable ambient store, and use explicit --server with CICADA_TOKEN.
   Assert selected contract registration, no bootstrapped singletons/watches, and a counter finishes under the worker
   id. Stop the executor via SIGTERM, assert exit 0, and clean containers/network on failure too, including anonymous
   volumes from the image's retained VOLUME /data (docker rm -v or equivalent). Retain existing
   default-image bootstrap tests. No port-specific JS/Kotlin test harness or portable fixture changes.
9. **Docs ownership.** Write docs-handoff.md for docs/guide/app.md's Sample service block: both invocations, bootstrap
    duties, full command surface, legacy constructor arguments, client/server tokens and removal of the node-id
    restriction. Request operations' HTTP-worker deployment guidance and e2e/README.md changes from the docs owner,
    with image entrypoint/health-check/ambient-store commands and
    the new acceptance class. Include the sample public module comments in source changes. Existing links remain;
    both consumer skills keep routing to the App/operations guidance, adding any route only if the docs owner adds a
    heading. No new guarantee row; retain all guarantees and their tests. The builder verifies the docs session's
    prose has landed before landing this change, with docs/doctests/skill checks passing; it does not edit that prose.
10. **Delivery.** Five implementation tasks cover identity, shared App definitions, HTTP checks, image/E2E, and
    handoff/verification/release preparation, followed by the full committed PostgreSQL/Docker gate. The independent
    heartbeat catalog/snapshot addition is the successor sample-heartbeat-watch; neither it nor NodeDownMonitor
    hosting or native HTTP-worker node-table heartbeats belong here. No contract snapshot changes in this change.
    The builder records actual checks; these design assertions are not claims of completed runtime validation.
