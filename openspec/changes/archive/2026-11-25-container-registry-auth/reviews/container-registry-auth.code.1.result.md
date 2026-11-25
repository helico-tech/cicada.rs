Verdict: fix needed

## Blocking

1. Two spec scenarios have no test that does what their WHEN says (`specs/container-docker/spec.md`).
   - "A login made after connect": every runtime in `e2e/tests/suite/container_docker.rs` (`create_from`, ~line 785) is made after `login_and_push`. The unit tests in `auth.rs` re-read `config.json` through `lookup`, but no runtime is involved. To see it: move the read into `docker_config`/`connect` and cache a `Config` on `DockerRuntime`. Every test still passes. Fixed: the E2E step makes the runtime with `docker_config(login)` before `login_and_push` and pulls with that same runtime afterwards.
   - "A public image with a broken helper": no test calls `create` while `credsStore` names a missing helper. `a_configured_helper_means_auths_is_not_used` stops at the `Lookup`. Fixed: `a_missing_image_is_pulled_and_a_repeated_create_needs_none` (or a new step) creates `PULLED` with a runtime whose folder holds `{"credsStore": "cicada-no-such-helper"}` and checks that the container is created. As a bonus, that step stops reading the developer's real `~/.docker` and keychain helper.

## Notes

1. "Credentials from a helper" is proven in parts (`helper()` picks `credsStore`, `run_helper` parses the answer, a configured helper skips `auths`). The success branch of `Config::credentials` with a helper is never run. A test for it would need the helper's folder passed in instead of `PATH`.
2. `config_dir` uses only `HOME`. On a Windows host (the backend supports the pipe), the CLI's `USERPROFILE` is not read, so there are no credentials unless `DOCKER_CONFIG` or `docker_config` is set. This follows the design, but the handoff could name it as a caveat.
3. The departures (an upper-case first part is a registry, as Docker parses it; "exited with a signal") are sound. A `credHelpers` entry of `""` falls back to `auths`, as the CLI does.
4. The task 5 tick and gate evidence are still open. The requester said not to block on them.

## Not checked

I ran no tests (no unit, E2E or gate run). I did not verify that `REGISTRY_HTPASSWD` matches the password, or the recorded `ImageNotFound` kind against `registry:2`. I did not check the CLI's helper key matching against real Docker Desktop or OrbStack helpers.
