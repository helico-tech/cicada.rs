Verdict: not yet

## Blocking

1. **Error paths without a scenario or test** (`specs/container-docker/spec.md`, ADDED requirement; design "Tests"). Five failures are named, two have scenarios. Missing: (a) a unix endpoint whose socket is missing gives `DaemonUnavailable`, naming context and endpoint. It is the only retryable path, and the design's test list leaves it out. (b) `currentContext` names a context without a meta file: `Other`, naming `currentContext`. (c) `DOCKER_CONTEXT` names a non-default context but there is no config folder (no `DOCKER_CONFIG`, no `HOME`): not decided. Fix: scenarios for (a) and (b); decide (c) (I would pick `Other`, as for a missing meta file); unit tests for all three.

## Notes

1. Empty variables: the CLI and `auth::config_dir` treat `DOCKER_CONTEXT=""` as unset. Write "set and not empty".
2. E2E agreement gets weaker. With a live `/var/run/docker.sock` and another `currentContext`, the backend uses the socket, while `docker_blocking` and `info`'s `docker version` use the context. Acceptable, but the helper's doc ("the daemon the rest of the suite reaches through the CLI") must change with it.
3. Rustdoc of `connect()` ("or at the local socket") and of `connect_to` ("such as the endpoint of the current Docker CLI context") must change too, not only the struct line.
4. On a server whose socket appears after the host starts, a `currentContext` can pick another daemon depending on timing. Rare; decision 1 accepts it.
5. Add `dep:sha2` to the `docker` feature.

## Questions for the owner

None.

## Not checked

Nothing built or run. I checked the meta path, the sha256 of `orbstack` and the `meta.json` shape against this machine's `~/.docker`, and read bollard 0.21.1's `connect_with_host` and its socket check. I did not check the CLI's TLS rule in the docker/cli source; I went from what I know of it.
