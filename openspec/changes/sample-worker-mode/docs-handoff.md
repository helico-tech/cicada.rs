# Docs handoff for sample-worker-mode

Version: 0.20.3. Refresh the dependency pins of `docs/how-to/install.md` once it is published. Not breaking for
library users: only the sample service changes. The libraries, actor contracts (existing contract bytes and snapshots
are unchanged), wire formats, the store schema, App's resolution rules and the guarantees are those of 0.20.2.

## What changed

- A second binary of the sample package and image, `cicada-sample-executor` (`Sample::executor` in
  `sample-service/src/app.rs`, `sample-service/src/bin/cicada-sample-executor.rs`). Same catalog, routes,
  `API_TOKEN` protection and variables as `cicada-sample-service`, App's whole command line (help, version under
  its own name, operator commands, `serve`, `worker` on a store, `worker --server`), but no `on_start` hook. So
  `cicada-sample-executor worker --server <url> --types <names>` runs the sample's types over HTTP; the original
  binary still refuses `worker --server` (ADR 0042, exit 2, names `App::on_start`).
- No bootstrap in any executor mode: no sample singletons (of `maintenance.cleanup`, `pricing`, `payments`,
  `registries.devices` and `cicada.dead-letters`) and no wildcard dead-letter watch. A node of the original sample
  hosting those types, or an API caller, supplies what a workload needs. Registration of contracts alone creates no
  actor.
- `sample-service/src/app.rs`: `run_process(definition)` is the shared `main` of both binaries (telemetry, then
  `App::run_async`); `app::Definition` names the two constructors.
- `Cargo.toml` of the sample: `default-run = "cicada-sample-service"`, so `cargo run -p cicada-sample-service -- serve`
  still works; the executor is `cargo run -p cicada-sample-service --bin cicada-sample-executor -- ...`.
- Identity: `SlowActor`, `CounterActor`, `RecorderActor` and the `EffectLog` record `Turn::node_id`, the claim owner
  (a node's id on the store, a worker's id over HTTP, never the server's). Their constructors and `Catalog::new` /
  `Catalog::with_fakes` keep the node-id argument for source compatibility and ignore it. The sample no longer sets
  `node_config` and no longer checks the id at start: App alone resolves it (`--node-id`, then `CICADA_NODE_ID`, then
  random). A `--node-id` that differs from `CICADA_NODE_ID` now starts and wins; before 0.20.3 it failed startup.
  `config::node_id` stays public but neither definition uses it.
- Image (`sample-service/Dockerfile`): both binaries in `/usr/local/bin`; entry point, user, volume, defaults, stop
  signal and health check unchanged.
- Tests: `sample-service/tests/e2e_actors.rs` (paused `TestActors`, legacy ids vs the runtime id);
  `sample-service/tests/sample_service.rs`: `a_node_id_flag_overrides_the_variable_in_records_and_effects` (replaces
  `a_node_id_flag_other_than_the_variable_fails_the_start`), `the_executor_serves_the_catalog_and_routes_without_bootstrap`,
  `the_executor_works_on_the_store_without_bootstrap`, `the_executor_runs_the_sample_over_http_without_a_database_or_bootstrap`,
  `the_executor_keeps_the_token_target_and_type_rules_and_the_sample_refuses_http`,
  `the_executor_binary_has_the_app_help_and_version_and_the_node_stays_the_default`; E2E class `SampleExecutorTest`
  in `e2e/tests/suite/sample_executor.rs` (order 9, host port 9095, network `cicada-rs-e2e-executor`).
- No guarantee row changes. No skill route changes are needed unless a new heading is added (see 4).

## 1. App guide, the sample service block

**Where.** `docs/guide/app.md`, `Sample service`.
**Text.** Keep the `serve` command. Replace "Use `CICADA_NODE_ID` consistently; a differing `--node-id` fails startup."
with: the E2E actors record the id of the node or worker that ran each turn; `--node-id` wins over `CICADA_NODE_ID`.
Add the second binary:

```sh
cargo run -p cicada-sample-service --bin cicada-sample-executor -- worker --server http://localhost:8080 --types e2e.counter
```

with the substance: `cicada-sample-executor` hosts the same types and routes with App's whole command line, but creates
no singletons and no dead-letters watch in any mode, so it can run as an HTTP worker; a node of `cicada-sample-service`
hosting those types, or an API caller, creates what a workload needs. `cicada-sample-service` still refuses
`worker --server` because of its startup hook. The worker sends `CICADA_TOKEN` (or `--token`); `API_TOKEN` protects a
served API and is never used as the client token. The constructors of the E2E actors and of `Catalog` still take a
node id and ignore it.
**Caveats.** Do not suggest that registration creates singletons. The HTTP worker has no HTTP listener, so its
`EffectLog` (`/e2e/effects`) cannot be read; read the actors' durable state on the server. No database-free build:
the package still compiles both SQL features, the HTTP path just never opens a store.

## 2. Operations, deploying the sample image as an HTTP worker

**Where.** `docs/operations.md`, `Deploying` (after the alternative worker without database credentials), or a short
paragraph of its own.
**Text.** The sample image runs the executor as an HTTP worker by overriding its entry point and turning off the HTTP
health check, which asks a server's `/ready`:

```sh
docker run --entrypoint /usr/local/bin/cicada-sample-executor --no-healthcheck \
  -e CICADA_TOKEN="$TOKEN" cicada-rs-sample \
  worker --server http://actors:8080 --types e2e.counter --node-id worker-1
```

In Compose: `entrypoint: ["/usr/local/bin/cicada-sample-executor"]`, `command: [worker, --server, ...]` and
`healthcheck: { disable: true }`. Give the token through `CICADA_TOKEN`, not in the arguments. An explicit `--server`
wins over the image's `CICADA_STORE`; `--store` together with `--server` is a usage error; `CICADA_SERVER` alone
(without the flag) loses to the image's `CICADA_STORE`, so pass `--server`. SIGTERM drains the worker and it exits 0.
**Caveats.** The image's `VOLUME /data` still makes an anonymous volume for a worker container; remove it with
`docker rm -v` (the worker never writes to it). The node id is trusted metadata, not proof of one physical process.

## 3. E2E README

**Where.** `e2e/README.md`, the suite description, the free ports in `Run the suite`, and the class order.
**Text.** Add the class `SampleExecutorTest` (after the SQLite node): an authenticated send-only server (`--types
none`, `API_TOKEN`, host port 9095) and `cicada-sample-executor worker --server` from the same image on the network
`cicada-rs-e2e-executor`, with the health check off, no database volume, an unreachable `CICADA_STORE`, `CICADA_TOKEN`
and `--node-id exec-worker`. It checks that only the selected types are registered, no singleton or watch exists, a
counter runs under `exec-worker`, and SIGTERM ends the worker with exit code 0. Its logs are saved as
`executor-<time>.log`; teardown removes both containers with their anonymous volumes and the network, also after a
failure. Port 9095 must be free (9092 is avoided as a common Kafka port).
**Caveats.** None.

## 4. Consumer skills

**Where.** `skills/cicada/SKILL.md`, `skills/actor-design/SKILL.md`.
**Text.** No change is needed: they already route to the App guide and operations. Add a route only if 1 or 2 adds a
new heading.
**Caveats.** None.

## Blocking

None. No doctest, link, guarantee row or skill route fails: the changed prose is advisory (the stale sentence about a
differing `--node-id` in `docs/guide/app.md` is now wrong but checked by no test). Landing waits for the docs session's
prose of 1–3, as the design asks.

## Status

Proposed text only; none of 1–3 is written yet. Checks run on this branch are listed in the build report and the gate
evidence.
