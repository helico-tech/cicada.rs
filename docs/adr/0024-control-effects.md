# 0024. Control effects

Date: 2026-11-12

## Status

Accepted

## Context

ADR 0018 lets a turn pause, resume, purge and delete another actor, atomically with its other effects, and fixes
that deletes come before creates in one commit, so that a turn can restart an actor. It leaves open what happens to
a turn that is running on a deleted actor, and says nothing of an effect on an actor that is not there, of an actor
that deletes itself or an ancestor, or of who may control whom. The change `control-effects` settles these. The
commit gets new parts, which changes a guarantee; the project rule wants that recorded.

## Decision

- **Each effect means what the same action from outside a turn means.** A pause stops new turns and lets a running
  one finish. A purge ends every pending message as `purged`, except one that is being handled. A delete removes
  the actor, its mailbox and its descendants (ADR 0019) and tells its watchers (ADR 0023), with the acting actor as
  `by`.
- **A delete by a turn voids a turn that is running on the target.** That turn commits nothing and its message ends
  `purged`. The delete does not wait: a supervisor must be able to remove an actor that is stuck.
- **An effect on an actor that does not exist does nothing.** It does not fail the commit: an actor can stop itself
  at any moment, and a controlling turn must not fail for that. A send to an actor that does not exist still
  rejects the commit.
- **A turn cannot delete itself or an ancestor.** It stops itself with `turn.stop()`. A delete of an ancestor
  rejects the commit. It may pause, resume and purge itself.
- **A turn may control any actor.** There is no check of ownership: actors are the trusted code of one system.
- **A fixed order in one commit:** the watches of the turn, the outcome, the cancels, the deletes, the creates, the
  purges, the pauses and resumes, the sends, the state, the notifications. Deletes before creates give the restart.
  A delete after a create of the same address is refused at the call. After a pause and a resume of one actor in
  one turn, the last call stands.
- **Several removals in one commit:** an actor that the turn names in a delete reads `deleted`, the turn's own actor
  reads `stopped` when it stops, and every other removed actor reads `parent`.
- **This is not the forced release that ADR 0015 refused.** ADR 0015 would not take a lease away from a running
  turn, because a second handler could run for the same message. A delete ends the lease with the actor: the
  message of that turn ends `purged` and is never handled again, and the actor that is created afterwards is
  another life. A lease ends only with its turn or with its actor.
- **The guarantees say so:**
  - G1: an instance is one life of an address, from its create to its removal. A handler of an earlier life may
    still run for a moment after the actor was deleted and created again; it commits nothing.
  - G2: the control of other actors is one of the parts of a commit that happen all together or not at all.
  - S7: a commit can end the pending messages of another actor, by a purge or a delete.

## Consequences

- An actor that manages others can stop and restart one in the same commit as its own state.
- Any actor can delete any other, as any actor can send anything to any other.
- A mistyped address in a control effect is not noticed: the effect does nothing.
- A restart by another actor than the parent makes a child independent (ADR 0019): a parent restarts its own
  children, with `create_child`.
- After a restart, the handler of the old life can overlap with the first handler of the new one in what it does
  outside the store.
- No schema change and no contract change: nodes before and after can share a database.
