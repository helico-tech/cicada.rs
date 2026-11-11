# Proposal

## Why

A turn can change only its own actor: its state, its sends and creates, cancels of its own messages, and its own
stop. Pause, resume, purge and delete of another actor exist only for operators (ADR 0015), outside any commit. An
actor that manages others, such as a supervisor, cannot stop or restart one atomically with its own state. ADR 0018
adds control effects.

## What Changes

- A turn can pause, resume, purge and delete another actor. Each is buffered like a send and applied by the turn's
  commit, together with the turn's other effects or not at all.
- No effect writes another actor's state.
- To restart an actor, a turn deletes it and creates it again. In one commit, deletes are applied before creates.
- A delete removes the actor's children as well (ADR 0019), as an operator delete does.

The simplest version: the four actions that operators already have, with the same meaning.

To decide in the design:

- What happens to a turn that runs on an actor while another turn deletes it (ADR 0018, Left to the changes). An
  operator delete today voids the lease: the running turn commits nothing and its message ends `purged`.
- What a control effect on an absent actor does: nothing, or reject the commit as a send to an absent actor does.
- A turn that deletes itself, or one of its own ancestors, this way.
- The order in which PostgreSQL locks the actors that a commit controls.

## Capabilities

### New Capabilities

- `control-effects`: a turn can pause, resume, purge and delete another actor, atomically with its other effects.

### Modified Capabilities

None. The operator actions of `mailbox-management` keep their requirements.

## Out of scope

- Writing or reading another actor's state.
- Cancelling another actor's pending messages or its running turn.
- A create by address and JSON state; `cicada-actors` says why a supervisor may need one.

## Order

Step 6 of ADR 0018. After `watches`, before `turn-observer`. It comes after `watches` so that a delete by a turn
notifies watchers from the start.

## Impact

- `crates/cicada` (`store` commit, `turn`, `runtime`), both stores, the store conformance suite, `README.md`,
  `docs/architecture.md` (effects, commit order, turn semantics) and `CHANGELOG.md`.
- The commit changes, so G2 gets new parts; ADR 0018 covers it.
