# Docs handoff for container-info

Version: 0.21.1. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` gains one read call on the runtime trait: `ContainerRuntime::info() -> Result<RuntimeInfo,
ContainerError>`. `RuntimeInfo` (`#[non_exhaustive]`, public `arch` and `os`, `RuntimeInfo::new(arch, os)`) holds the
runtime's processor kind and operating system in the spelling of image platforms (`amd64`, `arm64`, `arm`; `linux`),
so a host compares them with an image's platform as they are. The call changes nothing and is safe to repeat; it also
shows that the runtime answers. The trait's default fails with `Unsupported`, so a runtime written outside this repo
compiles unchanged; `ErrorKind::Unsupported`'s rustdoc now names `info` with the other added methods.

- `DockerRuntime::info` asks `GET /version` and returns its `Arch` and `Os` as Docker gives them. No connection or a
  timeout is the retryable `DaemonUnavailable`; a missing or empty `Arch` or `Os` is `Other`. No time limit of its own:
  bollard's request timeout (120 s) applies.
- `FakeRuntime::info` answers `amd64` on `linux` until `set_info(RuntimeInfo)`; `Call::Info` and `Method::Info` record
  the call and script failures with `fail_next` and `fail_always`.

Proved by the unit tests `info_answers_amd64_on_linux_until_a_test_sets_another_machine`,
`info_fails_as_scripted_and_records_every_call`, `a_repeated_info_changes_nothing` and
`a_runtime_without_logs_since_copy_out_make_dirs_or_info_compiles_and_fails_with_unsupported` (`src/fake.rs`),
`runtime_info_takes_arch_and_os_as_docker_gives_them` and `a_missing_or_empty_arch_or_os_is_other`
(`src/docker/map.rs`), `info_without_a_daemon_is_daemon_unavailable_twice` (`src/docker/mod.rs`), and the E2E step
`info_gives_linux_and_the_arch_the_cli_prints` in `e2e/tests/suite/container_docker.rs`, which compares with
`docker version --format '{{.Server.Arch}}'`.

## 1. The container runtime: asking the machine

**Where.** No guide covers `cicada-containers` yet; the page the docs session picks for the container runtime, under a
heading such as `The runtime's machine`.
**Text.** `runtime.info().await` gives a `RuntimeInfo` with `arch` (`amd64`, `arm64`, `arm`) and `os` (`linux`), in
the spelling of image platforms, as Docker's `GET /version` gives them. Use it to pick the runs or images that fit the
machine, and as a liveness check: a host that says it is here only while its runtime answers calls `info` each round.
A sketch (not compiled; the E2E step above is the real use):

```rust
match runtime.info().await {
    Ok(info) => pool.send(Here { arch: info.arch.clone() }),     // "arm64" on Apple silicon
    Err(e) if e.kind == ErrorKind::Unsupported => return Err(e),  // a runtime without info
    Err(_) => { /* Docker does not answer, or answers with an error: no Here this round */ }
}
```

**Caveats.** Every error but `Unsupported` means "not here this round": an unreachable Docker is `DaemonUnavailable`,
but one that answers and is not healthy (a 5xx, such as a Docker Desktop engine that is starting, or a Docker below the
API floor, Docker 25) is `Other`, as for every call. The call has no time limit of its own; bollard gives up after
120 seconds, so wrap it in `tokio::time::timeout` for a fast answer. Docker gives no variant: `arm` does not say `v6`
or `v7`. The names are Go's (`amd64`, `arm64`), not Rust's `std::env::consts::ARCH` (`x86_64`, `aarch64`). In tests,
`FakeRuntime` answers `amd64` on `linux` until `set_info`.

## Blocking

None.

## Added after release (from Factory, 2026-11-25)

A host that wraps a `DockerRuntime` in its own `ContainerRuntime` (to add logging, metrics or a pin) must forward
`info()` (and every other method with a default: `logs_since`, `copy_out`, `make_dirs`) to the inner runtime; otherwise
the trait's default answers `Unsupported`. Worth one line next to `info` in the guide.

## Status

No protected prose was edited; the text above is proposed. Checks run: see the build report
(`cargo test -p cicada-examples --test docs` and `--test skill`, and the gate).
