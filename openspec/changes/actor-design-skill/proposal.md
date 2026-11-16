# Proposal

## Why

A guide supplies knowledge, but a consumer's agent also needs a method for turning the user's problem into an
actor design. The owner approved the split on 2026-11-16: actor-design-guide supplies the knowledge first, this
change adds the interactive design skill, and actor-design-review later adds review of existing systems.

The research is retained here:

- [research/actor-design.md](research/actor-design.md): the design idioms and nine-step method, with sources.
- [research/cicada-fit.md](research/cicada-fit.md): the idioms checked against Cicada 0.18.11.
- `research/raw/`: the independent research runs and source checks.

The guide change consumes that research. This skill routes to the guide in the consumer's resolved Cicada source;
it does not copy the knowledge or treat this research as a consumer manual (ADR 0041).

## What Changes

- **A second consumer skill, actor-design**, holds the design method and routes to the design guide. The owner chose
  a method in the skill and knowledge in the guide on 2026-11-15; this change keeps that boundary.
- **Work with the user one step at a time** (owner, 2026-11-16). Draw requirements out of the user through concrete
  stories, such as an order whose payment times out. Follow the research's nine steps, a few questions per turn.
  The user's vocabulary names actors and messages. End each step with its written result and an updated drawing,
  confirmed by the user before advancing. Record assumptions as open questions and allow return to earlier steps.
- **Keep a visual design document** (owner, 2026-11-16). Save the results and Mermaid diagrams in the consumer's
  repository: an actor/message/watch map, sequence diagrams per story with failure branches, and state diagrams for
  actors with a lifecycle. It renders in editors and on GitHub and supports resuming the design across sessions.
- **Ship both consumer skills together** (owner, 2026-11-16). Extend the release archive to carry cicada and
  actor-design, with checked provenance. The root plugin already discovers every skill under skills/. Keep the
  existing skill available for building and operating consumers; the new one focuses on their design.
- **Check the workflow and distribution.** Extend skill route/version/evaluation and archive checks, along with the
  release driver's archive verification. Cover old or missing guides without silently using the latest source.
  Provide installation/update guidance through a docs handoff and record the second skill in an ADR extending 0041.

To decide in the design:

- How to resolve the guide: use the sibling cicada resolver or carry a copy, including missing-sibling behavior.
- The design document's default path, sections, update/resume behavior and preserving the user's existing work.
- Packaging both skills: member layout, provenance, version checks and upgrade compatibility with existing archives.
- The scope of each skill's description and the checks/evaluation cases that prove the step-by-step interaction.

## Out of scope

Writing the design guide or its live-handle example (actor-design-guide). Reviewing an existing system
(actor-design-review). Changes to Cicada execution, APIs, guarantees or wire formats. Actor pinning.

## Order

After `actor-design-guide`. Before `actor-design-review`.

## Impact

`skills/actor-design/`, consumer skill routes/version checks and evaluations, `scripts/package-skill.py`, shared
archive helpers as needed, and the release driver's archive verification/tests. An ADR and docs handoff for
installation and discovery. The docs session owns handwritten guidance; source, skill files and tests remain with
this change. No library API, guarantee, wire format or schema changes.
