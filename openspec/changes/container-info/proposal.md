# Proposal

## Why

Factory (cicada-code-factory) moves its container runs fully onto `cicada-containers` 0.21.0. One piece of its own
Docker code is left: its pool member says `Here` to Factory's pool only while its Docker answers, and sends the
processor kind as Docker reports it (`docker version` Server.Arch: `arm64`, `amd64`), which decides which runs fit the
machine. The runtime trait has no call for this (request from the Factory session, 2026-11-24).

## What Changes

- `ContainerRuntime::info() -> Result<RuntimeInfo, ContainerError>`, with `RuntimeInfo { arch, os }`
  (`#[non_exhaustive]`, with a constructor); `arch` in the spelling images use (`amd64`, `arm64`, `arm`), as Docker's
  `/version` gives it. The default fails with `Unsupported`, like the other added calls.
- `DockerRuntime`: `GET /version`, which also proves the daemon answers (`DaemonUnavailable` when it does not).
- `FakeRuntime`: scripted with `set_info`, recorded as `Call::Info` / `Method::Info`.

## Out of scope

A daemon version or other `/version` fields (Factory needs none). Health checks beyond one call. The actor.

## Order

After `pool-size-check`. No following change.

## Impact

`crates/cicada-containers` (trait, fake, Docker backend, its E2E class). Additive: an added method with a default, so a
patch after 0.21.0 (0.21.1).
