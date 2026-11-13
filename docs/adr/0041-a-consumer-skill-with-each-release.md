# 0041. A consumer skill with each release

Date: 2026-11-13

## Status

Accepted for `cicada-agent-skill`. The owner requested a maintained skill for projects using Cicada, and delegated
technical choices to an Opus 5.5 adviser. This records a release convention, not a change to a runtime guarantee.

## Context

Cicada's guides and examples are checked with its code. A separately maintained agent manual would drift, and a
current manual can recommend APIs absent from an older pinned dependency. The contributor loops are not a workflow
for consumer projects. Cargo's resolved git source currently includes the documentation and examples of that commit.

## Decision

- Maintain a portable consumer skill at skills/cicada: a short SKILL.md and a Python standard-library source resolver.
  The skill routes; the guides own the knowledge. Reusable actor-design advice goes into the core guide.
- Resolve the consumer's packages with locked offline Cargo metadata and explicit target/features. Use exact package
  names and returned manifest paths, never guessed Cargo cache paths or shortened commit ids. Report multiple source
  groups, missing sources and dirty/path/fork provenance rather than choosing a convenient upstream version.
- Use source-relative routing entries that remain meaningful after installation elsewhere. The resolved dependency's
  guides and code take precedence over the router's version. A missing heading calls for inspection, not a claim that
  an API does not exist. Do not teach unreleased changes as stable functionality.
- Record skill version in frontmatter and release provenance in SOURCE.json. Distinguish numerical version equality
  from revision identity and local modifications. Discovery neither fetches nor changes dependencies or installed skills.
- Publish cicada-skill-vX.Y.Z.tar.gz from the immutable tag commit alongside each successful GitHub Release. It contains
  the skill, resolver and source manifest, without a second copy of the docs. Packaging uses Git objects and is checked
  before release creation; versions, archive paths and provenance must agree.
- Document installation in a consumer's chosen project skill directory and explicit updates after comparing local
  changes. Older releases without an asset can use a newer router against their own resolved source. Do not modify
  existing release tags or imply an installed skill has updated automatically.
- Check source discovery fixtures, routes, version, package provenance and the existing guide doctests in CI. Use
  representative consumer-task evaluations in review to assess behavior; no external model service is required by CI.

## Consequences

- Skill and library releases move together, while old consumers continue reading their actual dependency's docs.
  Source checkouts supply offline knowledge; a vendor/registry tree without docs falls back to exact source/rustdoc.
  Offline users without matching sources receive a clear limitation and can still do work supported by local evidence.
- All layers are discoverable without loading every guide into an agent's initial context. Changes to docs can improve
  advice without duplicating rules in the skill. New user-facing capabilities still require a maintenance review.
- Automated checks detect structural drift, not universal correctness or completeness. Installation/update remains
  explicit; no background updater, credential setup, forced dependency upgrades or contributor workflow is imposed.
- The helper needs Python and installed Cargo/Rust tools; partial manual discovery remains possible without them.
  Target filtering and chosen features are reported, and do not claim to cover every consumer build configuration.
