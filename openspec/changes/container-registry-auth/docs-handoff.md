# Docs handoff for container-registry-auth

Version: 0.21.1. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`DockerRuntime::create` pulls a missing image with the machine's `docker login`, as the `docker` CLI does. Before,
it pulled without credentials, so a private image had to be on the machine already. Nothing changes in the spec, the
store or the `ContainerRuntime` trait.

- New: `DockerRuntime::docker_config(self, dir) -> Self`, a builder method that sets the folder of `config.json`.
- At each pull (only when the image is not on the daemon) the runtime reads `config.json` from the folder set with
  `docker_config`, else from `DOCKER_CONFIG`, else from `$HOME/.docker`. A `docker login` after `connect` counts.
- For the image's registry it takes, in the CLI's order: the helper that `credHelpers` names, else the `credsStore`
  helper, else the `auths` entry (`auth`, base64 of `user:password`, and `identitytoken`). When a helper is
  configured, `auths` is not used for that registry.
- A helper runs as `docker-credential-<name> get` from `PATH`, with the server on stdin, and is killed after 10 s.
- A missing or broken file, a registry without an entry, and a helper that is missing, fails, gives no JSON or times
  out all give a pull without credentials, so a public image still works.
- A failed pull's message ends with one of `(credentials from docker-credential-<name>)`,
  `(credentials from config.json)`, `(no credentials for <registry>)` or
  `(no credentials: docker-credential-<name> <reason>)` (`not found`, `exited with <code>`, `timed out`,
  `gave no JSON`). Never a credential or a helper's output. The error kinds stay as they were.

Proved by the unit tests in `crates/cicada-containers/src/docker/auth.rs` (the registry of an image, key
normalisation, the order, `auth` decoding, helper scripts that answer, give `<token>`, exit non-zero, give no JSON or
sleep over the limit, a missing and a broken file), `a_failed_pull_names_its_credential_source_and_never_a_credential`
and `debug_shows_no_client` in `src/docker/mod.rs`, and the E2E step
`a_private_image_is_pulled_with_the_login_of_config_json` in `e2e/tests/suite/container_docker.rs` (a `registry:2` with
basic auth; the CLI logs in and pushes with its own `--config` folder, after the runtime that pulls was made), and
`a_public_image_is_pulled_past_a_helper_that_is_not_installed` (a public image pulled while `credsStore` names a missing
helper). The unit test `a_configured_helper_gives_the_credentials_instead_of_auths` runs a helper through the lookup. Against that registry, Docker reports a pull
without a login as "pull access denied … no basic auth credentials", which maps to `ImageNotFound`.

## 1. The container runtime: private images

**Where.** No guide covers `cicada-containers` yet; the page the docs session picks for the container runtime (next to
`The runtime's machine` from `container-info`), under a heading such as `Private images`.
**Text.** `DockerRuntime` pulls a missing image with the machine's Docker login, so `docker login <registry>` on the
machine is enough. It reads `config.json` at each pull, as the CLI does: from the folder set with `docker_config`, else
`DOCKER_CONFIG`, else `~/.docker`. Credentials come from the `credHelpers` helper for the registry, else the
`credsStore` helper (Docker Desktop and OrbStack keep logins there), else the `auths` entry. A sketch, from the rustdoc
of `DockerRuntime` and the design:

```rust
let runtime = DockerRuntime::connect()?;                                    // $DOCKER_CONFIG or ~/.docker, per pull
let runtime = DockerRuntime::connect()?.docker_config("/srv/factory/.docker"); // a service's own folder
```

**Caveats.** Run `docker login` as the user the host runs as: a service whose `$HOME` is not that user's reads another
`~/.docker`; give it the folder with `docker_config` or `DOCKER_CONFIG`. Without `DOCKER_CONFIG` or `docker_config`, the runtime looks
only in `$HOME/.docker`: on a Windows host it does not read the profile folder (`USERPROFILE`) as the CLI does, so set
one of the two there. A helper must be on the host's `PATH` and
answer within 10 seconds; a keychain helper that waits for a prompt is killed and the pull runs without credentials.
A failing source never fails the create by itself: a public image still pulls. A refused login keeps its kind: Docker
Hub and `registry:2` say "pull access denied" (`ImageNotFound`), others answer "unauthorized" or "denied" (`Other`,
which the container actor retries). Read the end of the message to see whether credentials were found. Cicada does not
log in, and credentials never go into the spec, the store, a log line or an error.

## Blocking

None.

## Status

No protected prose was edited; the text above is proposed. Checks run: see the build report
(`cargo test -p cicada-examples --test docs` and `--test skill`, and the gate with E2E).
