# Proposal

## Why

A turn can change only its own actor: its state, its sends and creates, cancels of its own messages, and its own
stop. Pause, resume, purge and delete of another actor exist only for operators (ADR 0015), outside any commit. An
actor that manages others, such as a supervisor, cannot stop or restart one atomically with its own state. ADR 0018
adds control effects.

## What Changes

- **BREAKING** A turn can pause, resume, purge and delete another actor, named by a typed ref or an address. Each is
  buffered like a send and applied by the turn's commit, together with the turn's other effects or not at all. This
  adds to the guarantee G2.
- No effect writes another actor's state.
- To restart an actor, a turn deletes it and creates it again. In one commit, deletes are applied before creates,
  and purges, pauses and resumes before sends.
- Each effect means what the same action from outside a turn means. A delete removes the actor's children as well
  (ADR 0019), voids a turn that is running on it, and notifies its watchers.
- An effect on an actor that does not exist does nothing; it does not fail the commit.
- A turn cannot delete itself (it stops itself) or one of its ancestors.
- A turn may control any actor, not only the ones it created.

The simplest version: the four actions that operators already have, with the same meaning.

## Capabilities

### New Capabilities

- `control-effects`: a turn can pause, resume, purge and delete another actor, atomically with its other effects.

### Modified Capabilities

- `randomized-testing`: the comparison of the stores builds control effects, and the simulation under faults has an
  actor that restarts another.
- `watches`: a watched actor can also go away by the delete of another actor's turn; its `Stopped` names that actor.

The operator actions of `mailbox-management` keep their requirements.

## Out of scope

- Writing or reading another actor's state.
- Cancelling another actor's pending messages or its running turn.
- A create by address and JSON state; `cicada-actors` says why a supervisor may need one.

## Order

Step 6 of ADR 0018. After `watches`, before `turn-observer`. It comes after `watches` so that a delete by a turn
notifies watchers from the start.

## Impact

- `crates/cicada` (`store` commit, `turn`, `runtime`), both stores, the store conformance suite, the randomised
  comparison of the stores, `README.md`,
  `docs/architecture.md` (G1, G2, S7, effects, commit order, turn semantics), `docs/operations.md` (what a restart
  does to a running handler, the log of control effects) and `CHANGELOG.md`.
- The commit changes, so G2 gets new parts, and G1 and S7 get clearer words. ADR 0024 records the rules that ADR
  0018 left open and the new wording, and says why a restart is not the forced release that ADR 0015 refused; the
  sentence on leases in `docs/architecture.md` is made exact.
