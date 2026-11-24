# Proposal

## Why

Factory moved its container runs onto `ContainerActor` and `DockerRuntime` 0.21.0 (Factory ADR 0009). Its design review
found one regression against its old `docker` CLI backend: `DockerRuntime::create` pulls a missing image without
credentials, so an image from a private registry (Factory's GitLab registry) that is not on the machine yet ends
`ImageNotFound`, which Factory reports as "failed to start". The CLI used the machine's `docker login` (request from
the Factory session, 2026-11-24; Factory issue 0017). Until this lands, Factory pre-pulls private images.

## What Changes

- `DockerRuntime` pulls with the machine's Docker credentials: it reads Docker's own `config.json` (the `auths`
  entries, and the credential helpers `credsStore` / `credHelpers`, since Docker Desktop and OrbStack keep logins in
  a helper), and passes the credentials for the image's registry to the pull. So `docker login` on the machine is
  enough. Nothing goes into the spec or the store.

To decide in the design:

- Which config file: `DOCKER_CONFIG`, then `~/.docker/config.json`, as the CLI does; when it is read (at `connect`, or
  at each pull so a later `docker login` counts).
- Credential helpers: run `docker-credential-<name> get` with the registry on stdin, as the CLI does, with a time
  limit; what a missing or failing helper gives (a pull without credentials, so a public image still works).
- The registry name of an image (Docker Hub's `docker.io` / `index.docker.io` spelling, a host with a port).
- Secrets: credentials never in a log line, an error or `Debug`.
- Not taken unless the design finds (a) cannot work: a per-pull `ContainerSpec::registry_auth` built by the job.

## Out of scope

Registry login from Cicada. Image builds. Other runtimes than Docker. Credentials in the store or the spec.

## Order

After `container-info`. No following change.

## Impact

The Docker backend of `crates/cicada-containers` and its E2E class (a private registry in the test: a local
`registry:2` with basic auth, if it can be set up). Additive, a patch (0.21.1 with `container-info`).
