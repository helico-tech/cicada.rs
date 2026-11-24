# 0056. Shared layers for external resources in optional crates

Date: 2026-11-24

## Status

Accepted for container-runtime. Supersedes in part [ADR 0011](0011-library-only.md): its rule that container handles
live with the application that needs them. Its removal of the console and of `cicada-docker` stands.

## Context

ADR 0011 removed `cicada-docker` because it was an application of the library, not a part of it: an actor type that
manages something outside Cicada lives with the application that needs it.

Since then two projects on Cicada each wrote the same layer for Docker containers. cicada-code-factory has a `Runner`
trait over `bollard` with a fake (about 600 lines); brood has a `ContainerRuntime` trait over the `docker` CLI with a
fake (about 800 lines). Both had to get the same details right: a container found by a fixed name so a repeated call
is safe, removal of leftovers by label, how a container ended, secrets kept off the command line, and a fake for tests
without Docker. Issue #2 (higher-level actors) asks for "external resource reconciliation" checked against a second
adapter; these are the two.

The reasons of ADR 0011 still hold for the core: building the library must not need Docker, and the core must not
grow a dependency for one kind of application.

## Decision

- When two or more consumers need the same layer for an external resource, that layer may live in this workspace as
  an optional crate outside the core, such as `cicada-containers`.
- The core (`cicada`) and the other library crates never depend on such a crate; only examples and tests of the
  workspace may. A consumer adds it by itself.
- The crate keeps heavy or platform dependencies (a Docker client) behind features, and has a fake behind the feature
  `testing`, so a consumer's tests and this workspace's tests run without the external system.
- A layer with one consumer stays with that consumer, as ADR 0011 says.

## Consequences

- `cicada-containers` comes back in a new form: a runtime trait first (`container-runtime`), its Docker backend
  (`container-docker`), then a ready-made container actor (`container-actor`).
- The gate checks the crate with the rest of the workspace; a check of the dependency trees keeps the library crates
  free of it.
- The crate shares the workspace version and its releases, so a change to it is released like any other.
- Each further crate of this kind needs two consumers, named in its proposal.
