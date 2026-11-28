# 0076. A native macOS inspector

Date: 2026-11-27

## Status

Accepted for inspector-window. Supersedes in part [ADR 0011](0011-library-only.md): its rule that the repository holds
no console, for a native macOS console only. Its removal of the browser console (`cicada-ui`) stands, and so does
[ADR 0056](0056-shared-external-resource-crates.md)'s exception for container layers. Builds on ADR 0074 (the tray).
Superseded by [ADR 0078](0078-remove-the-inspector.md): the inspector was removed before it shipped.

## Context

ADR 0011 removed the browser console because every build that depended on it needed Node.js, npm and the network, and
because its wire types were kept in step with the Rust API by hand. Operators still want to look inside a running system
from a Mac with a native window, opened on its own or from the tray, not only through `cicada` and its terminal UI. The
tray (ADR 0074) showed that a macOS app in Rust, with AppKit through `objc2`, builds with Cargo alone and keeps its
mapping from API answers to what it shows in plain, tested code.

## Decision

- The repository holds a native macOS inspector in its own crate, `cicada-inspector`, built in Rust on the HTTP API
  through `cicada-client`, and on `cicada-app` for the contexts of `cicada config`. It needs no other toolchain than
  Rust, and uses the API's own Rust types, so no wire type is kept by hand.
- Its model (what it fetches and how it maps answers) is portable and tested on every platform against an in-process
  server; its AppKit view is behind `cfg(target_os = "macos")` and holds every `unsafe` call.
- No library crate depends on `cicada-inspector`, with one allowed edge: the optional feature `inspector` of
  `cicada-tray`, which opens the window from the tray's menu. The gate's check of the dependency trees names that
  exception.
- There is still no browser console in this repository, and no console served by a node.

## Consequences

- Building the workspace still needs only Rust; the inspector builds its window on macOS only and its model everywhere.
- The inspector shows only what the HTTP API serves: what it cannot see over HTTP (the retention of failed outcomes, for
  example) it does not show.
- A browser console, or an inspector for Windows or Linux, still lives in a repository of its own.
