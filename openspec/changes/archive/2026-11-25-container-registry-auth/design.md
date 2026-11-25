# Design

`DockerRuntime::create` pulls a missing image without credentials, so a private image that is not on the machine
fails to start (`ImageNotFound`, or `Other` for a registry that answers with a 500). The `docker` CLI pulls with the
machine's `docker login`; the backend does the same, read the same way the CLI reads it.
Nothing goes into the spec, the store or the trait.

```rust
let runtime = DockerRuntime::connect()?;          // reads $DOCKER_CONFIG/config.json or ~/.docker/config.json per pull
let runtime = DockerRuntime::connect()?.docker_config("/srv/factory/.docker");   // a service's own folder
```

## Decisions

1. **The CLI's file, read at each pull.** The folder is the one set with `docker_config(dir)`, else `DOCKER_CONFIG`,
   else `$HOME/.docker`; the file is `config.json` in it. It is read only when a pull is needed (the image is not on
   the daemon), so a `docker login` after `connect` counts, and a host whose images are all present never reads it. A
   missing or unreadable file, or one that is not JSON, means a pull without credentials. Not taken: reading once at
   `connect` (a later login would need a restart; a pull is slow anyway); a `registry_auth` on `ContainerSpec` (the
   proposal's fallback: credentials in the job's state, and every host would build them).
2. **`docker_config(dir)` on `DockerRuntime`.** A builder method that sets the folder. For a host running as a
   service whose `$HOME` is not the user's, and for the E2E test, which must not change the environment of a
   parallel test run. Not taken: only the environment (setting `DOCKER_CONFIG` inside a test process races other
   tests).
3. **The registry of an image, as Docker parses it.** The first part of the reference before a `/` is a registry
   when it holds a `.` or a `:` or is `localhost` (`registry.gitlab.com`, `localhost:5000`); otherwise the image is on
   Docker Hub. `docker.io`, `index.docker.io` and `registry-1.docker.io` are Docker Hub too. Docker Hub's key is
   `https://index.docker.io/v1/`, as the CLI writes it. Keys of `auths` and `credHelpers` are compared after the
   CLI's normalisation: drop `http://` or `https://` and everything from the first `/`, so `https://ghcr.io` and
   `ghcr.io` match; Docker Hub's spellings all match Docker Hub.
4. **Where the credentials come from, as the CLI decides.** A helper named in `credHelpers` for the registry, else
   the `credsStore` helper, else the `auths` entry. A helper is run as `docker-credential-<name> get`, found on
   `PATH`, with the server key on stdin; its JSON answer gives `Username` and `Secret`, and a `Username` of
   `<token>` makes the secret an identity token. An `auths` entry gives `auth` (base64 of `user:password`, split at
   the first `:`) and `identitytoken`. When a helper is configured, `auths` is not used for that registry, as in the
   CLI (Docker Desktop leaves empty `auths` entries next to its helper).
5. **A failing helper means no credentials, with a 10 s limit.** A helper that is missing, exits non-zero (also
   "credentials not found"), answers no JSON or takes over 10 s (it is killed) gives a pull without credentials, so a
   public image still works, as the CLI ignores the error too. The limit is fixed: long enough for a helper that
   calls a cloud (ECR), short enough that a stuck keychain prompt does not hang a create. Not taken: an option for
   the limit (nobody asked); failing the create (a public image would fail for a broken helper).
6. **A failed pull says where its credentials came from, never what they are.** The message of a failed pull ends
   with one of: `(credentials from docker-credential-<name>)`, `(credentials from config.json)`,
   `(no credentials for <registry>)` or `(no credentials: docker-credential-<name> <reason>)`, where the reason is
   `not found`, `exited with <code>`, `timed out` or `gave no JSON`. The helper's output, stderr included, never goes
   into a message. Credentials live only inside the pull call: never logged, in an error or in `Debug`
   (`DockerRuntime`'s `Debug` already prints only `negotiated`, and the folder is not secret). Not taken: an
   unchanged message (an `unauthorized` pull would not say whether a login was found).
7. **Own small reader; `base64` 0.22 as a dependency of the `docker` feature.** About 150 lines in
   `src/docker/auth.rs` with `serde_json` and `tokio::process`. `base64` 0.22 is already in the tree through bollard.
   Not taken: the `docker_credential` crate (blocking `std::process`, no time limit, its own error messages).
8. **A refused login keeps its kind.** Docker reports a pull without valid credentials as "pull access denied … does
   not exist" (Docker Hub: `ImageNotFound`, as today) or as "no basic auth credentials", "unauthorized" or "denied"
   (often a 500: `Other`, which the container actor retries with growing waits). That stays: a retry can cure an
   expired token that the helper renews on the next pull (ECR), and a bare `denied` would also catch "permission
   denied" on the daemon's storage. The message from decision 6 says whether credentials were found (design review,
   blocking 1). Not taken: mapping every refused login to `ImageNotFound` (no retry, a spec change for one cause).

## Tests

- Unit (`auth.rs`): the registry of `alpine`, `library/alpine`, `docker.io/x`, `ghcr.io/o/x`, `localhost:5000/x`,
  `host:5000/x@sha256:…`; key normalisation; the order `credHelpers`, `credsStore`, `auths`; `auth` decoding with a
  `:` in the password; a helper written as a script in a temp folder: an answer, `<token>`, a non-zero exit, no JSON,
  and a sleep over the limit. The helper runner takes its limit as an argument, so the tests use a short real-time
  limit: a paused clock would fire the limit at once while the child process runs. A missing file and a broken file
  give no credentials.
- E2E (`e2e/tests/suite/container_docker.rs`): a `registry:2` container with htpasswd basic auth on a free host port;
  an image pushed to it with `docker --config <tempdir> login` and `push`, never the developer's own `~/.docker` or
  keychain; a runtime with `docker_config(dir)` whose `config.json` has an `auths` entry for `localhost:<port>`
  creates a container from that image after `docker rmi`; the same runtime with an empty folder fails, and the
  message says `no credentials for localhost:<port>` (the kind is whatever Docker's answer maps to; the test records
  it). Everything removed after. If the daemon cannot reach the registry on `localhost` (a VM-based Docker), the
  builder reports it instead of dropping the case.
- Rustdoc: the `DockerRuntime` doc that says a private image must be local (`src/docker/mod.rs`, near line 46) gives
  the credential rules instead.

## Docs

Nothing breaks. The handoff gives the container guide the credential rules (the file, the order, a failing helper,
`docker_config`) and the hint to run `docker login` as the user the host runs as.
