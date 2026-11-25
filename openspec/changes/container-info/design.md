# Design

Factory's pool member says `Here` only while its Docker answers, and sends the processor kind Docker reports, which
decides which runs fit the machine. One read call on the runtime gives both.

```rust
#[non_exhaustive]
pub struct RuntimeInfo { pub arch: String, pub os: String }   // RuntimeInfo::new(arch, os)
// ContainerRuntime, after sweep():
async fn info(&self) -> Result<RuntimeInfo, ContainerError> { /* default: Unsupported */ }
```

```rust
match runtime.info().await {
    Ok(info) => pool.send(Here { arch: info.arch.clone() }),     // "arm64" on Apple silicon
    Err(e) if e.kind == ErrorKind::Unsupported => return Err(e),  // a runtime without info
    Err(_) => { /* Docker does not answer, or answers with an error: no Here this round */ }
}
```

## Decisions

1. **A new method with a default.** `info()` on `ContainerRuntime`, whose default fails with `Unsupported`, as
   `container-limits-and-kinds` decided for added methods: a runtime outside this repo compiles unchanged, and the
   change is a patch (0.21.1). The doc of `ErrorKind::Unsupported` names it next to the others. It is a read: safe to
   repeat, with no effect. Not taken: a method on `DockerRuntime` only (Factory holds an `Arc<dyn ContainerRuntime>`
   and tests it on the fake).
2. **Strings as Docker gives them.** `arch` and `os` are Docker's `/version` fields `Arch` and `Os`, which are Go's
   names (`amd64`, `arm64`, `arm`, `linux`): the spelling of image platforms, so Factory compares them with an
   image's platform as they are. Not taken: an enum (a closed list breaks on the next processor kind and needs a
   mapping both ways); Rust's names from `std::env::consts` (`x86_64`, `aarch64`: not what images use). Limit: Docker
   gives no variant, so `arm` does not say `v6` or `v7`.
3. **Fields public, with a constructor.** `RuntimeInfo` is `#[non_exhaustive]` with public fields and
   `RuntimeInfo::new(arch, os)`, like `ContainerSummary`: a later field (a variant, a daemon version) is added without
   a break, and another runtime builds one with `new`. Not taken: accessor methods (no other type of the crate uses
   them).
4. **Docker: `GET /version` on the agreed client.** `DockerRuntime::info` gets the client through `docker()` (which
   agrees the API version on the first call and retries that after a failure) and calls bollard's `version()`. A
   failed connection or a timeout is `DaemonUnavailable` through `kind_of`, so `is_retryable()` tells "Docker does not
   answer". A missing or empty `Arch` or `Os` is `Other` ("docker /version gave no Arch"). No own time limit: bollard's
   request timeout (120 s) applies; a host that wants a faster answer wraps the call in `tokio::time::timeout`. The
   rustdoc says so, names `/version`, and says which kinds an answering but unhealthy Docker gives: a 5xx (a Docker
   Desktop engine that is starting) is `Other`, as for every call, and so is a Docker below the API floor (Docker 25).
   A liveness check therefore treats every error but `Unsupported` as "not here this round" (design review, notes 1
   and 5). Not taken: `DaemonUnavailable` for a 5xx of `info` only (the kinds would differ between calls). Not taken: `/info` (a heavier call that lists containers and images; its `Architecture` uses the
   kernel's names, `x86_64` and `aarch64`); a time limit argument (a new knob nobody asked for).
5. **The fake answers `amd64` on `linux` until told otherwise.** `FakeRuntime::set_info(RuntimeInfo)` sets the
   answer; `info()` records `Call::Info` and can fail through `fail_next(Method::Info, kind)` and `fail_always`, so a
   test scripts a Docker that stops answering and comes back. Not taken: the host's processor (tests would differ
   per machine, and in Rust's spelling).

## Tests

- Unit: the default fails with `Unsupported` for a runtime without `info`; the fake's default answer, `set_info`, a
  scripted failure and its call record; `DockerRuntime` on a socket without a daemon gives `DaemonUnavailable`, twice;
  the map from a `SystemVersion` with a missing or empty `Arch` or `Os` to `Other`.
- E2E (`e2e/tests/suite/container_docker.rs`): `info()` gives `os` `linux` and the `arch` that
  `docker version --format '{{.Server.Arch}}'` prints for the same context.

## Docs

Nothing breaks: no doc block, link, guarantee row or skill route uses the trait's method list. No guide covers
`cicada-containers` yet; the handoff gives the docs session, for the page it picks, `info`, `RuntimeInfo`, the spelling of `arch` and the hint to wrap the call in a timeout for a fast
answer, and that every error but `Unsupported` means "not here" for a liveness check.
