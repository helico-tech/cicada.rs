# 0078. Remove the inspector

Date: 2026-11-28

## Status

Accepted for remove-inspector. Supersedes [ADR 0076](0076-a-native-macos-inspector.md). [ADR 0011](0011-library-only.md)
holds again, apart from [ADR 0056](0056-shared-external-resource-crates.md)'s container layers; the tray keeps
[ADR 0074](0074-a-tray-crate-next-to-the-app.md).

## Context

ADR 0076 added `cicada-inspector`, a native macOS window over the HTTP API, and let the tray open it through an optional
feature. On 2026-11-28 the owner ruled the inspector out of scope. It exists only in the unreleased 0.26.0: no published
version has it, so no user depends on it.

## Decision

- The repository holds no inspector. `cicada-inspector` and the tray's feature `inspector` are removed before 0.26.0.
- The tray (ADR 0074) and the terminal UI of `cicada-app` stay; operators look inside a system with `cicada`.
- `ActorClient::changes` (ADR 0077) stays as a client feature of its own.

## Consequences

- No library crate may depend on `cicada-tray` or `cicada-containers`; the inspector's exception is gone.
- The changelog of 0.26.0 does not name the inspector, as it never shipped.
- A console, native or in a browser, again lives in a repository of its own.
