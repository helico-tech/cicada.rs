# End-to-end and fault tests

The sample service (`sample-service`) in Docker, driven over HTTP only:

- `docker-compose.yml` — compose project `cicada-rs-e2e`: PostgreSQL 18 (host port 25432), `node-1`..`node-3` hosting
  every type (9081–9083) and a send-only node (9084), all from the image `cicada-rs-sample:e2e`. `LEASE_TTL=5s`,
  `POLL_INTERVAL=200ms`, `SHUTDOWN_GRACE_PERIOD=5s`. The ports differ from the Kotlin stack's, so both can run at once.
- `tests/suite/` — the automated suite (port of Kotlin's `e2e-tests`: T13 fault tests, T17 E2E) against that stack,
  plus single SQLite nodes started with `docker run` (9090, and 9091 with `API_TOKEN`). One module per Kotlin test
  class; `src/` is the harness (docker CLI, nodes, polling, ordering checks, the suite runner).

## Run the suite

Needs a running Docker (OrbStack, Docker Desktop, …) with the `docker compose` plugin, and free ports 9081–9084, 9090,
9091 and 25432.

```sh
CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture
```

Without `CICADA_E2E=1` the suite's one test returns at once, so `cargo test --workspace` needs no Docker.

The test builds the image (`docker build -f sample-service/Dockerfile -t cicada-rs-sample:e2e .`), removes leftovers of
an aborted run, starts the compose stack once, runs the classes in the Kotlin order (API, examples, ordering under
load, kill / pause / stop a node mid-turn, outcomes while a node is killed, PostgreSQL stopped and started again, full
cluster restart, SQLite node), and always removes every container and volume afterwards: at the end of the run, while
unwinding from a panic, and on Ctrl-C. Every test is bounded by 4 minutes. A run takes about 5 minutes (plus the first
image build, which compiles the workspace in Docker). `--nocapture` shows progress (`E2E: PASS …` per test); without it
the output appears only when the suite fails.

Container logs of every run are saved to `target/e2e-logs/` before teardown (`compose-<time>.log`,
`sqlite-<time>.log`).

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
curl -s -X PUT localhost:9084/actors/e2e/slow/demo -H 'Content-Type: application/json' -d '{"state":{}}'
curl -s -X POST localhost:9084/actors/e2e/slow/demo/messages -H 'Content-Type: application/json' \
  -d '{"type":"Sleep","payload":{"millis":8000,"label":"demo"}}'          # note the "id"
curl -s localhost:9084/messages/<id>                                       # "status":"running","node":"node-2"
docker compose -f e2e/docker-compose.yml kill -s SIGKILL node-2            # or: pause node-2 / stop node-2
curl -s localhost:9084/messages/<id>                                       # later: "done", "attempt":2
docker compose -f e2e/docker-compose.yml up -d --wait                      # bring node-2 back (unpause for pause)
```

Shorter or longer timings: `LEASE_TTL=10s docker compose -f e2e/docker-compose.yml up -d --wait`.

## Clean up

After an aborted run, or when done driving by hand:

```sh
docker compose -p cicada-rs-e2e -f e2e/docker-compose.yml down -v --remove-orphans
docker rm -f -v cicada-rs-e2e-sqlite cicada-rs-e2e-auth
docker volume rm -f cicada-rs-e2e-sqlite-data
```
