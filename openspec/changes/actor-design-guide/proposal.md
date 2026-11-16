# Proposal

## Why

The API guides explain how Cicada works, but a consumer also needs to decide which actors exist, what each owns,
which messages they exchange, how they recover and how they meet outside systems. The owner approved three small
changes on 2026-11-16: this guide, the interactive design skill, then its review mode. Knowledge comes first so both
workflows can route to the same checked guidance, following ADR 0041.

The research remains under `openspec/changes/actor-design-skill/research/`: `actor-design.md` merges the independent
research runs, and `cicada-fit.md` checks the idioms against 0.18.11. Its claims must be checked against the source
that ships the guide; it is research input, not a replacement for docs/architecture.md.

## What Changes

- **A design guide in docs/guide/** holds the knowledge: the questions from problem to design, actor boundaries and
  identities, protocols, coordination and time, failure, outside effects, a pattern chooser and pitfalls with fixes.
  The core guide keeps API instruction and links to this guide. Reuse existing worked examples and API references.
- **Concrete design artifacts.** Show responsibility cards, message/state tables and the nine steps' written results.
  Use actor maps, sequence diagrams with failure branches, and lifecycle state diagrams, in Mermaid. The future
  skill uses these forms; this change does not implement the interactive workflow or its output-file convention.
- **Live handles get a worked answer.** Teach the two forms already chosen by the owner: a turn uses a handle kept
  by its host, or a host task follows the actor's durable state and reports observations. The first works today with
  one node hosting the type. The second gets a checked worked example in this change and needs no actor pin.
  The later pinned-actors change owns the many-node form of the first and its child-process example.
- **Smaller research findings stay guidance.** Cover application-database writes as outside effects, reply routes,
  internal-message checks, bounded deferred work, state tables and recovery patterns through existing capabilities.
  No new runtime feature is introduced to fill a guidance gap.
- **ADR and documentation coordination.** Extend ADR 0041's rule that design knowledge lives in the core guide:
  give the expanded knowledge its own guide while the skills keep the method and routes. The docs session writes
  the prose from the builder's handoff. The builder owns the worked example, tests, doctest wiring and skill routes.

Decided in the design:

- `docs/guide/design.md` uses a generic idiom followed by its Cicada form (E1, owner, 2026-11-16).
- The websocket session example (F2, owner, 2026-11-16): bounded outgoing frames, acknowledgements and reconnects;
  refuse new output with BufferFull when full (G1), and one logical client per session, with separate keys for
  independent tabs/devices (H1), both chosen by the owner on 2026-11-16. The design specifies cumulative acks,
  bounded replay, client cursor handling and the failure tests; these are example conventions, not new core guarantees.
- The design names the smaller findings, navigation, skill routes and guide doctest wiring. The new guide and its
  links require docs-owner preparation before landing; the handoff names G2's row for the acceptance/rollback tests.

## Out of scope

The new actor-design skill, its packaging and interactive design/review behavior. Runtime, wire-format, schema or
guarantee changes. Actor pinning and the child-process example. A survey of every actor framework or pattern.

## Order

After `skill-plugin`. Before `actor-design-skill`.

## Impact

A new guide and its navigation, prepared by the docs session. `examples/` for one worked example, virtual-time
checks and guide doctests; `skills/cicada/SKILL.md` for routing and `examples/tests/skill.rs` for coverage. An ADR
extending 0041 and a docs handoff. No library API, guarantee, wire format or schema changes.
