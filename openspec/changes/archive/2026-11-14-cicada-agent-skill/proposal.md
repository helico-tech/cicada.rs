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
- Read authoritative references from the resolved dependency's source rather than packaging another manual. Include
  source/version provenance, offline behavior and an explicit path to update the installed skill.
- Check packaging, references and example compilation in CI/release checks, and evaluate representative consumer
  requests to catch misleading guidance that structural validation cannot prove correct.

## Settled decisions

- Maintain skills/cicada/SKILL.md and a read-only Python source resolver; ship them as a versioned release archive.
- Resolve installed Cargo packages offline with the lockfile, using manifest paths instead of guessed cache locations.
  Report mixed or unavailable sources and distinguish version equality from revision equality.
- Route to the resolved source's guides/examples/API. Missing headings require further inspection, not an invented
  API or an automatic conclusion that a feature is absent. No copied manuals or silent fetch/update.
- Put reusable actor-design advice in the core guide, preserve contract/deployment caveats and test guide examples.
- Check source resolution, routes and archive provenance in CI; use focused behavioral review for quality. No check
  claims to prove every best practice or automatically refresh installed copies.

## Capabilities

### New Capabilities

- `consumer-skill`: a portable, version-aware coding-agent skill for projects using Cicada.

## Out of scope

Implementing a consumer's application, publishing a plugin marketplace integration, silently upgrading dependencies
or overwriting installed skills, a background updater, maintaining guidance for Kotlin, or changing Cicada runtime
semantics. This does not impose Cicada's contributor loops or release workflow on consumer repositories.

## Order

After `listener-probe-on-close`. Before `build-loop-docs-handoff`.

## Impact

A public skill source/package, generation and validation tooling, CI/release artifacts and installation documentation.
Existing guides, examples and architecture remain authoritative. No runtime API, database schema or wire change.
