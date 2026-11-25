Verdict: not yet

## Blocking

1. **Decision 8 changes behaviour without a delta, and the Why does not need it.** Where: design.md decision 8; the spec delta only ADDs. The live requirement "Docker's errors map onto error kinds" makes a refused login `Other`. The actor retries `Other` with backoff (`actor/mod.rs` ~L513). As `ImageNotFound`, the job ends `failed`/`notStarted` at once. Retries that would cure it are lost, such as ECR's "denied: … token has expired", which the helper renews on the next pull. A bare `denied` also matches "permission denied" from storage. My pick: drop decision 8, because decision 6's message already explains the failure. If it stays: add a MODIFIED delta, name the actor's new outcome, and drop the bare `denied`.
2. **Error paths without scenarios.** Where: spec "A failing credential source…". There is no scenario for a helper that times out or exits non-zero. If decision 8 stays, there is also none for wrong credentials refused by the registry. Add one scenario each.

## Notes

- The helper tests cannot run on virtual time. A paused Tokio clock moves forward while it waits on a real child process, so the timeout fires at once. Pass the limit internally and use a short real-time one.
- E2E: a push to the htpasswd registry needs a login. Use `docker --config <tempdir>` or the API, never the developer's `~/.docker` or keychain. A fixed bcrypt line saves pulling an htpasswd image.
- The rustdoc at `docker/mod.rs:46` says "a private image must be local". It becomes wrong, so name it in the tasks.
- No scenario covers credentials from a helper, the main case the Why names (Docker Desktop, OrbStack).
- Bollard 0.21.1 does not log headers, so `X-Registry-Auth` stays out of logs.

## Questions for the owner

- Q1: Did Factory's GitLab pull really end `ImageNotFound`? If yes, Factory does not need decision 8.

## Not checked

Docker's real error texts for GitLab and ECR. Whether the daemon reaches a `localhost` registry under OrbStack or Docker Desktop.
