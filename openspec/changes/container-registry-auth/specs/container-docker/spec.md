## ADDED Requirements

### Requirement: A pull uses the machine's Docker login

When `create` must pull an image, `DockerRuntime` SHALL read Docker's `config.json` from the folder set with
`docker_config`, else `DOCKER_CONFIG`, else `$HOME/.docker`, and SHALL pass the credentials for the image's registry
to the pull: from the helper named in `credHelpers` for that registry, else from the `credsStore` helper, else from the
`auths` entry. The registry SHALL be found as Docker finds it, with Docker Hub under `https://index.docker.io/v1/`.

#### Scenario: A private image after docker login

- **WHEN** an image is only in a registry with basic auth, `config.json` has an `auths` entry for that registry, and
  `create` is called for a container of that image
- **THEN** the image is pulled and the container is created

#### Scenario: Credentials from a helper

- **WHEN** `credsStore` names a helper that answers with a user name and a secret for the image's registry, and
  `create` pulls an image from it
- **THEN** the pull uses that user name and secret, and the `auths` entry for that registry is not used

#### Scenario: A login made after connect

- **WHEN** a runtime is made, then a credential is written to `config.json`, then `create` pulls a private image
- **THEN** the pull uses that credential

### Requirement: A failing credential source means a pull without credentials

A missing or broken `config.json`, a registry without an entry, and a helper that is missing, exits non-zero, gives no
JSON or takes more than 10 seconds SHALL give a pull without credentials. A refused login SHALL keep the kind its
error maps to today. A failed pull's message SHALL say where its
credentials came from, or why there were none, and SHALL never hold a credential or a helper's output.

#### Scenario: A public image with a broken helper

- **WHEN** `credsStore` names a helper that is not installed and `create` pulls a public image
- **THEN** the pull runs without credentials and the container is created

#### Scenario: A helper that hangs or fails

- **WHEN** the helper for a registry takes longer than the limit, or exits non-zero, and `create` pulls an image
- **THEN** the helper is stopped, the pull runs without credentials, and a failed pull's message names the helper and
  `timed out` or the exit code

#### Scenario: A private image without a login

- **WHEN** `config.json` has no entry for a private registry and `create` pulls an image from it
- **THEN** it fails, and the message says that there were no credentials for that registry
