# 0051. An interactive actor-design skill alongside the coding skill

Date: 2026-11-16

## Status

Accepted for `actor-design-skill`. Extends ADRs 0041, 0049 and 0050. The owner delegated remaining design decisions
to the design session and selected Opus 5.5 for reviews on 2026-11-16.

## Context

The design guide holds knowledge, but an agent needs a method to elicit a consumer's requirements and retain a
design across sessions. The owner chose nine steps, user checkpoints and visual artifacts. A second skill must
ship alongside cicada without duplicating its source resolver or breaking reproducible historical archives.

## Decision

- Add actor-design with its method and a document template. Keep API knowledge in the resolved source's guide.
  Use the sibling cicada resolver with existing status/fallback behavior; disclose missing or mismatched siblings
  and distinguish resolver provenance from the new skill's metadata. Never infer APIs from the latest source.
- Ask at most three questions at a time, one step at a time, with written results and evolving Mermaid diagrams.
  Default to confirmation before advancing; honor explicit consumer delegation and record its scope and reasons.
  Do not invent requirements, actors or confirmation. The existing cicada skill retains coding/operations tasks.
- Default the saved result to docs/design/actors.md subject to user paths and repository rules. Preserve existing
  work, progress, open questions and decisions; resume from those and mark dependent sections when an earlier
  decision changes. No automatic implementation, commit or deployment. Review mode is a separate later change.
- Keep the archive name and cicada/ paths. Add an actor-design/ sibling with its template and identical SOURCE.json
  provenance. A tag-owned scripts/skill-archive-format marker selects format 2, requiring both skills. Absence means
  legacy only if no actor-design tree exists. This preserves historical bytes without arbitrary version cutoffs.
- Require the marker and both versions in current release preflight; select and check archive layout from the
  pinned commit, including every provenance file. Keep immutable tags, recovery digests and fixture assets intact.
- Check structure, sources, packaging and staged evaluation cases locally; behavioral review evaluates the method.
  The docs session owns installation, entry-point and release guidance through the producer's handoff.

## Consequences

Consumers receive one source resolver and two complementary methods in the same optional plugin or archive. A
manually installed actor-design skill needs its sibling; missing source knowledge is an explicit limitation.
The archive format gains a small explicit marker, not a general packaging framework. No runtime guarantee, library
API, wire format or schema changes. Consumer decision delegation is explicit and never inferred from silence.
