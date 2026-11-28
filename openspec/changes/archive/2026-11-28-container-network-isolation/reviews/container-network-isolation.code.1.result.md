Verdict: pass

## Blocking findings

None.

## Notes

1. `e2e/tests/suite/container_docker.rs:1012-1013` (the isolated probe, comment at :992). An isolated bridge has no gateway, so IPAM gives the first address of the subnet to the first container. That container is the probe itself. I checked this on the local Docker 29.4: on an isolated internal network, the first container gets `x.x.x.1/24`, and `nc -z -w 3 x.x.x.1 39483` exits 1 because the probe connects to itself and is refused. The test still catches the main failure: if the daemon ignored the option, the bridge would hold `.1` and the probe would connect. But exit 1 does not come from the host refusing, and the comment ("Try the first usable address of its subnet") hides this. Suggestion: check that the probe's own address is not the one it dials, or start another container on the network first. Then the probe dials a free address or a peer, not itself. Also fix the comment.
2. `crates/cicada-containers/src/docker/mod.rs:490-493`: `ensure_network` calls `spec.check_isolation()`, and `map::check_network` runs it again. The first call is needed so the check runs before `docker()` (the design wants no Docker call for `InvalidSpec`). It could take only the version from `check_network`, or the comment could say why the check runs twice.
3. `crates/cicada-containers/src/fake.rs:495-498`: an invalid spec is recorded without `enter`, so a scripted failure of `EnsureNetwork` stays queued for the next call. This is fine, but not obvious; a one-line comment would help.

## Checked

Tasks 1-5 ticked, each with its work in the diff. Every scenario has a test: fake tests for the runtime spec, map and dead-socket tests for API 1.47 (the check runs before any inspect, so the old-daemon-with-options scenario is covered), and E2E for reachability. Design decisions 1-8 are followed. CHANGELOG Added line is under 0.26.0 unreleased. Versions are 0.26.0 in Cargo.toml, both skills and plugin.json. No protected docs edited. The handoff is true to the code. The builder's example compiles against the `non_exhaustive` struct's public fields.

## Not checked

I did not run the test suite or the gate (I relied on the builder's gate evidence). I did not run the E2E test itself; I only reproduced the address assignment by hand.
