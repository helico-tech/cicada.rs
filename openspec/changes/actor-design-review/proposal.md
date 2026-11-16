# Proposal

## Why

A consumer may already have an actor design or codebase and need to know whether its ownership, protocols and
failure behavior are sound. The owner approved a separate review-mode change on 2026-11-16, after the guide and
interactive design skill, so it can use their knowledge, artifact forms and resolved-source discovery.

## What Changes

- **Add review mode to the actor-design skill.** Accept an existing design or codebase and use the design guide's
  checklist to check actor boundaries, identities, state, protocols, coordination, time, failures and outside effects.
  Extend the existing skill rather than adding a third consumer skill.
- **Draw the system before judging it** (owner, 2026-11-16). Reconstruct actors, messages and watches from evidence,
  then draw the important stories and lifecycle states. Distinguish code behavior, documented intent and unknowns.
  Use the same Mermaid forms as design mode, with concrete failure traces where a rule can be broken.
- **Explain actionable findings.** Name the evidence, the consequence and a proposed correction. Ask the user for
  missing requirements instead of inventing them. The design guide owns the knowledge; the skill owns the method.
- **Check review behavior.** Extend the skill's evaluations with flawed and sound designs and incomplete evidence.
  Keep the existing archive/plugin distribution and resolved-source discipline. Hand off mode-selection guidance
  to the docs session.

To decide in the design:

- Whether review writes into the design-mode document or a separate report, and how it preserves an existing design.
- The review's stages, user checkpoints and finding format, including priority and evidence locations.
- How review distinguishes an intentional tradeoff from a defect when requirements or source evidence are missing.
- Representative cases that demonstrate useful findings without prescribing one universal actor decomposition.

## Out of scope

New design-mode behavior, a second copy of the guide, a third consumer skill, automatic fixes or a deployment.
Changes to Cicada runtime behavior, guarantees, wire formats or schema. Pinning actors.

## Order

After `actor-design-skill`. Before `pinned-actors`.

## Impact

The actor-design skill and its evaluations/checks, plus a docs handoff. Reuse the guide and source resolver already
introduced by earlier changes. No library API, guarantee, wire format or schema changes.
