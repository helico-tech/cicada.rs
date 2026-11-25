# End-to-end and fault tests

The sample service (`sample-service`) in Docker, driven over HTTP only:

- `docker-compose.yml` — compose project `cicada-rs-e2e`: PostgreSQL 18 (host port 25432), `node-1`..`node-3` hosting
  every type (9081–9083) and a send-only node (9084, `CICADA_TYPES=none`), all from the image `cicada-rs-sample:e2e`,
  whose entry point runs `serve`. `CICADA_LEASE_TTL=5s`, `CICADA_POLL_INTERVAL=200ms`,
  `CICADA_SHUTDOWN_GRACE_PERIOD=5s`.
- `tests/suite/` — the automated suite (fault tests and E2E) against that stack,
  plus single SQLite nodes started with `docker run` (9090, and 9091 with `API_TOKEN`). One module per test
  class; `src/` is the harness (docker CLI, nodes, polling, ordering checks, the suite runner).
- `SampleExecutorTest` runs ninth, after the SQLite class: an authenticated send-only server (`--types none`,
  `API_TOKEN`, host port 9095) and `cicada-sample-executor worker --server` from the same image, on their own network
  `cicada-rs-e2e-executor`. The executor has no HTTP health check or database mount; it gets an unreachable
  `CICADA_STORE`, its client token in `CICADA_TOKEN` and `--node-id exec-worker` overriding its environment id.
  The class checks selected-type registration, no sample singleton or watch, a counter recording `exec-worker`,
  and exit code 0 after SIGTERM. Teardown removes both containers, their anonymous volumes and the network, also
  after a failure.
- `ContainerDockerTest` runs tenth: `DockerRuntime` of `cicada-containers` against the daemon `connect()` finds, as
  the `docker` CLI does, so no `DOCKER_HOST` is needed. It uses `postgres:18`, pulls `busybox:1.37.0` (removed first when no container uses it) and runs a
  `registry:2` with basic auth for the private pull, so it needs registry access. Everything it makes is named
  `cicada-rs-e2e-ctr-*` and labelled `cicada-rs-e2e=containers`; its hooks sweep that label and the suite's teardown
  removes it. Run only this class with `CICADA_E2E_ONLY=ContainerDockerTest` (the stack still comes up first).

## Run the suite

Needs a running Docker (OrbStack, Docker Desktop, …) with the `docker compose` plugin, and free ports 9081–9084, 9090,
9091, 9095 and 25432. Port 9092 is avoided because Kafka commonly uses it.

```sh
CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture
```

Without `CICADA_E2E=1` the suite's one test returns at once, so `cargo test --workspace` needs no Docker.

The test builds the image (`docker build -f sample-service/Dockerfile -t cicada-rs-sample:e2e .`), removes leftovers of
an aborted run, starts the compose stack once, runs the classes in a fixed order (API, examples, ordering under
load, kill / pause / stop a node mid-turn, outcomes while a node is killed, PostgreSQL stopped and started again, full
cluster restart, SQLite node, sample HTTP executor, Docker containers), and always removes every container and volume afterwards:
at the end of the run, while unwinding from a panic, and on Ctrl-C. Every test is bounded by 4 minutes. A run takes about 3 minutes (170–180 s; plus the
first image build, which compiles the workspace in Docker). `--nocapture` shows progress (`E2E: PASS …` per test); without it
the output appears only when the suite fails.

Container logs of every run are saved to `target/e2e-logs/` before teardown (`compose-<time>.log`,
`sqlite-<time>.log`, `executor-<time>.log`).

To remove what an aborted container class left by hand:

```sh
docker rm -f -v $(docker ps -aq --filter label=cicada-rs-e2e=containers)
docker network rm $(docker network ls -q --filter label=cicada-rs-e2e=containers)
docker volume rm $(docker volume ls -q --filter label=cicada-rs-e2e=containers)
```

Useful variations:

```sh
CICADA_E2E=1 CICADA_E2E_ONLY=NodeFaultTest cargo test -p cicada-e2e -- --nocapture   # one class (the stack still comes up)
CICADA_E2E=1 CICADA_E2E_ONLY=NodeFaultTest::kill cargo test -p cicada-e2e -- --nocapture   # tests whose Class::test name contains this
CICADA_DOCKER=/path/to/docker CICADA_E2E=1 cargo test -p cicada-e2e                   # another docker CLI
```

## Drive the stack by hand

```sh
docker compose -f e2e/docker-compose.yml up -d --build --wait
curl -s localhost:9081/health
```

Fault scenarios (what the suite automates):

```sh
# A slow turn, then kill its node mid-turn; another node re-runs it after the lease TTL (attempt 2).
curl -s -X PUT localhost:9084/actors/e2e.slow/demo -H 'Content-Type: application/json' -d '{"state":{}}'
curl -s -X POST localhost:9084/actors/e2e.slow/demo/messages -H 'Content-Type: application/json' \
  -d '{"type":"Sleep","payload":{"millis":8000,"label":"demo"}}'          # note the "id"
curl -s localhost:9084/messages/<id>                                       # "status":"running","node":"node-2"
docker compose -f e2e/docker-compose.yml kill -s SIGKILL node-2            # or: pause node-2 / stop node-2
curl -s localhost:9084/messages/<id>                                       # later: "done", "attempt":2
docker compose -f e2e/docker-compose.yml up -d --wait                      # bring node-2 back (unpause for pause)
```

Shorter or longer timings: `LEASE_TTL=10s docker compose -f e2e/docker-compose.yml up -d --wait` (the compose file
passes the variable of the shell on as `CICADA_LEASE_TTL`; so do `POLL_INTERVAL` and `SHUTDOWN_GRACE_PERIOD`).

## Telemetry

`docker-compose.otel.yml` adds Grafana's all-in-one OpenTelemetry image and makes every node export traces and
metrics to it:

```sh
docker compose -f e2e/docker-compose.yml -f e2e/docker-compose.otel.yml up -d --build --wait
open http://localhost:3001     # Grafana: Explore → Tempo (traces) or Prometheus (cicada_turn_duration, cicada_message_lag)
docker compose -f e2e/docker-compose.yml -f e2e/docker-compose.otel.yml down -v
```

## Clean up

After an aborted run, or when done driving by hand:

```sh
docker compose -p cicada-rs-e2e -f e2e/docker-compose.yml down -v --remove-orphans
docker rm -f -v cicada-rs-e2e-sqlite cicada-rs-e2e-auth
docker rm -f -v cicada-rs-e2e-exec-server cicada-rs-e2e-executor
docker network rm -f cicada-rs-e2e-executor
docker volume rm -f cicada-rs-e2e-sqlite-data
```
