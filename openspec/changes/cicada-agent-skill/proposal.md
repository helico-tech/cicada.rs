# Proposal

## Why

People using Cicada in their own projects need their coding agents to understand its programming model, choose the
right layer and apply its operational practices. The repository's design/build/review skills serve contributors;
they are not a consumer guide. A separate public skill should make the maintained knowledge easy to find without
turning it into another manual that drifts from the library.

The owner asks for an agent skill that stays up to date and covers effective Cicada use. Up to date means guidance
verified against the project's resolved Cicada source/version, with new skill artifacts produced for each release.
An installed copy cannot promise it has silently updated, and latest-main advice must not override an older dependency.

## What Changes

- Publish a portable `cicada` skill for consumer projects, maintained in this repository with a short SKILL.md entry.
- Route from the task and resolved dependency to the matching guides, architecture, examples and API references;
  load detail as needed instead of placing every manual into the initial instructions.
- Cover project integration and layer choice, actor identity/state/messages, durable flows and ready-made actors,
  external-effect idempotency, retries and cancellation, virtual-time testing, HTTP/workers, telemetry and operations.
- Package authoritative references from the same source revision rather than hand-maintaining another copy. Include
  source/version provenance, offline behavior and an explicit path to update the installed skill.
- Check packaging, references and example compilation in CI/release checks, and evaluate representative consumer
  requests to catch misleading guidance that structural validation cannot prove correct.

## To decide

- Exact source layout and release/install format, keeping the skill usable outside a Cicada checkout and across
  agents that support SKILL.md without requiring a particular connector or agent harness.
- Version discovery for Cargo lockfiles, workspace dependencies, git revisions, local paths, mixed versions and new
  projects; source precedence and honest behavior when matching sources or network access are unavailable.
- Which references ship offline versus load from pinned upstream sources; deterministic generation and link handling.
- The minimum always-loaded practices, task routing and ownership of improvements to canonical guides.
- What CI can enforce automatically, what needs behavioral review, and release integration within five build tasks.

## Out of scope

Implementing a consumer's application, publishing a plugin marketplace integration, silently upgrading dependencies
or overwriting installed skills, a background updater, maintaining guidance for Kotlin, or changing Cicada runtime
semantics. This does not impose Cicada's contributor loops or release workflow on consumer repositories.

## Order

After `listener-probe-on-close`. No following change.

## Impact

A public skill source/package, generation and validation tooling, CI/release artifacts and installation documentation.
Existing guides, examples and architecture remain authoritative. No runtime API, database schema or wire change.
