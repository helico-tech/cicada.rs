# Proposal

## Why

The guides say how Cicada's API works. They say little about the step before it: how to cut a problem into actors.
Which actors exist, what each one owns, which messages they exchange, how they agree without a shared transaction,
how they fail, and how they meet outside services and processes. A consumer who works with an AI agent gets code
that compiles and a design that may be wrong: one actor that does everything, a rule split over two actors, an
outside call that is not safe to repeat.

The owner wants a skill, `actor-design`, that helps a consumer think a problem through in the actor-idiomatic way,
and that reviews an existing design against those idioms. It is about designing systems with actors, not about how
actors are executed: that is Cicada's job.

The research is done and sits in this folder (2026-11-15):

- [research/actor-design.md](research/actor-design.md): the design idioms, with sources. Two independent runs
  (Claude deep research, Codex), merged. It is Cicada-agnostic.
- [research/cicada-fit.md](research/cicada-fit.md): each idiom checked against Cicada 0.18.11.
- `research/raw/`: what each run returned, with the quotes from the sources.

## What Changes

The owner decided the shape on 2026-11-15: a skill that holds the method and routes to a guide that holds the
knowledge. The rule of ADR 0041 stays: a skill routes, the guides own the knowledge.

- **A design guide** in `docs/guide/` holds the knowledge: the questions to ask in order, a pattern chooser, the
  anti-patterns with their symptoms and fixes, and how each idiom is written in Cicada.
- **A second consumer skill, `actor-design`**, holds the method and routes to that guide. It has a design mode, the
  nine steps from a problem to a design, and a review mode, the checklist run against an existing design or
  codebase.
- **Live handles get a worked answer in the guide.** A process, a device or a socket cannot live in an actor. The
  guide teaches the two ways of `research/cicada-fit.md`: the turn uses a handle that the host keeps (the worked
  example of `pinned-actors`), and the machine follows the actor's state (for a websocket, no pin).

To decide in the design:

- **Generic or Cicada-specific.** The research is generic. The guide can teach the generic idiom and then its
  Cicada form, or only the Cicada form.
- **How the skill finds the guide** of the Cicada source that the project resolves: its own copy of the resolver
  of `skills/cicada`, or a dependency on that skill.
- **How it ships and how it is checked**: with each release like `skills/cicada` (`scripts/package-skill.py`), and
  what `examples/tests/skill.rs` checks for it.
- **How to split it.** Probably three changes: the guide, the skill's design mode, the review mode.
- **What the guide says about the smaller findings** of `research/cicada-fit.md`, such as a handler's write to the
  application's own tables.

## Out of scope

How Cicada executes actors. Changes to Cicada's behaviour. The one design-level gap the check found belongs to
`pinned-actors`; the smaller findings are guidance, not code.

## Order

After `sample-worker-mode`. No following change.

## Impact

Handwritten documentation and a consumer skill, both owned by the docs session: a docs handoff carries the text.
`skills/`, `scripts/package-skill.py` and `examples/tests/skill.rs` for the second skill. A short ADR that extends
ADR 0041: a second skill, and design advice in a guide of its own, not in the core guide. No change to a crate, a
guarantee or a wire format.
