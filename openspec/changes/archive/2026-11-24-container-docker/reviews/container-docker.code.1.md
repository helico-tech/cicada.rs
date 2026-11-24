---
id: container-docker.code.1
kind: code
change: container-docker
round: 1
base: 8e46164abfeb8a07d5f8c50e797f5c799df7c0e1
head: f58b9875d8f40137fed3ce30becb4b8e993d8be3
requested: 2026-11-24T11:24:32+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-docker.code.1 <file>`.

## Note from the requester

Version: 0.20.7 (the unreleased batch; no bump)

Builder report (head f58b987): tasks 1-5 done. cicada-containers unit tests 42; ContainerDockerTest (e2e/tests/suite/container_docker.rs) 15; whole E2E 46; cargo test --workspace with PostgreSQL 3163 passed; fmt, clippy -D warnings clean; feature-builds.sh, dependency-trees.sh, script tests 102, docs 11, skill 18, cargo deny, openspec strict pass; gate.py run --e2e full and e2e pass on f58b987, run without DOCKER_HOST.
Departures: the stop test uses busybox:1.37.0 (postgres stops with SIGINT, exit 130); stop sends the image's stop signal as docker stop does (change loop agreed). Added DockerRuntime::connect_to(host: &str) (DOCKER_HOST form, no bollard type) so the E2E class can use the current Docker context's endpoint when DOCKER_HOST is unset (change loop asked for this: the gate must pass without a hand-set DOCKER_HOST). An untagged image is pulled as :latest; an empty Attach list means no network; copy_in refuses a path on a read-only mount (fake too); connect() fails DaemonUnavailable when the socket is missing.
Docs: handed off, nothing blocks.
