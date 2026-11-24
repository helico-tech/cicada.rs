Verdict: pass

## Blocking findings

None.

## Notes

1. `docker/mod.rs` `connect()`: decision 1 says "no I/O", so a host can start while the daemon is down. But bollard checks that the socket file exists, so `connect()` fails when Docker Desktop is stopped and its socket is gone. The error is `DaemonUnavailable`, which can be retried, and the rustdoc and handoff say so. Still, a host that calls `connect()` at startup must retry it. Worth a line in the guide.
2. The fake and the backend disagree on a path that equals a mount target (`/work`). The fake's `under` counts it as on the mount. `map::uploads` needs a path strictly below the mount. With a read-only root, the fake accepts it and the backend refuses it. With a read-only mount at `/in`, the fake refuses `/in`, and the backend uploads it at `/`, where Docker fails.
3. `sweep`: a container that is already gone (404/409) is counted in `swept.containers`. A network or volume that is already gone is not counted.
4. `list` fails with `Other` when one container reports no state. Skipping that container would match how a vanished one is handled.
5. `connect_to` is new public API outside the design. The change loop agreed to it, it names no bollard type, and the changelog and handoff cover it.

Tasks 1–5 are ticked and their work is in the diff. Every scenario has a test (unit, `dependency-trees.sh` or `ContainerDockerTest`). The version stays 0.20.7. No protected docs changed. The handoff matches the code.

## Not checked

I ran no builds, tests or gate; I relied on the builder's evidence for f58b987. I did not check bollard 0.21.1's own behaviour (status codes, version negotiation, the socket check) beyond what the E2E results show. I did not check the Windows pipe.
