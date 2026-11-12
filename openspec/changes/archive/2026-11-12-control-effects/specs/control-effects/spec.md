# Spec Delta

## Purpose

A turn can pause, resume, purge and delete another actor, in the same commit as its own state and its other
effects, with the meaning these actions have when an operator does them.

## ADDED Requirements

### Requirement: A turn controls another actor in its own commit

A turn SHALL be able to pause, resume, purge and delete another actor, named by a typed ref or by an address. Each
SHALL be applied by the turn's commit together with its other effects, or not at all. No effect SHALL write another
actor's state.

#### Scenario: A pause and a resume

- **WHEN** a turn pauses an actor, and a message is sent to that actor, and a later turn resumes it
- **THEN** the message waits while the actor is paused, and is handled after the resume

#### Scenario: A purge

- **WHEN** a turn purges an actor that has three pending messages
- **THEN** the three messages end as `purged`, and the actor is still there with its state

#### Scenario: A delete

- **WHEN** a turn deletes an actor that has a pending message
- **THEN** the actor is gone and its message ends as `purged`

#### Scenario: A turn that fails

- **WHEN** a turn pauses one actor and deletes another, and then fails
- **THEN** the first is not paused and the second is still there

#### Scenario: By address

- **WHEN** a turn deletes an actor with `delete_at` and the address it read from a `Stopped` message
- **THEN** the actor is gone, as with `delete` and a ref

#### Scenario: A pause and a purge while a turn runs on the target

- **WHEN** a turn pauses and purges an actor on which another turn is running, with two more messages pending
- **THEN** the running turn finishes and commits, the two pending messages end as `purged`, and no new turn is
  claimed for the actor until it is resumed

#### Scenario: A resume wakes the node

- **WHEN** a turn resumes a paused actor that has a pending message, on a node that hosts it, on virtual time
- **THEN** the message is handled at once, not after the next poll

### Requirement: Effects of one commit are applied in a fixed order

A commit SHALL apply the watches of its turn first, then deletes before creates, creates before purges, pauses and
resumes, and those before sends.

#### Scenario: A restart

- **WHEN** a turn deletes an actor that has a state and a pending message, creates it again with a new state, and
  sends it a message, all in one turn
- **THEN** the actor has the new state, the old message ended as `purged`, and the new message is handled

#### Scenario: A parent restarts its child

- **WHEN** a turn of a parent deletes its child and creates it again with `create_child`, in one turn
- **THEN** the new actor is a child of that parent, and the descendants of the old one are gone

#### Scenario: Another actor restarts a child

- **WHEN** a turn of an actor that is not the parent deletes a child and creates it again, and the parent later
  calls `create_child` for it
- **THEN** the new actor has no parent, and the message of the parent's turn ends as `failed`

#### Scenario: A watch and a delete in one turn

- **WHEN** a turn watches an actor and deletes it in the same turn
- **THEN** the turn's own actor has one `Stopped` message for it

#### Scenario: Emptying a mailbox and filling it

- **WHEN** a turn purges an actor and sends it a message in the same turn
- **THEN** the messages from before end as `purged`, and the new message is handled

#### Scenario: Creating an actor that starts paused

- **WHEN** a turn creates an actor, pauses it and sends it a message in the same turn
- **THEN** the message waits until the actor is resumed

### Requirement: An effect on an actor that does not exist does nothing

A pause, a resume, a purge and a delete of an actor that does not exist SHALL change nothing and SHALL NOT fail the
commit.

#### Scenario: The target stopped itself

- **WHEN** a turn pauses, purges and deletes an actor that stopped itself just before
- **THEN** the commit succeeds and the turn's other effects are applied

#### Scenario: A commit that is rejected removes nothing

- **WHEN** a turn deletes one actor and sends to another that does not exist
- **THEN** the commit is rejected, the message of the turn ends as `failed`, and the first actor is still there

#### Scenario: A restart of an actor that is not there

- **WHEN** a turn deletes and creates an actor that does not exist and that a third actor watches by its address
- **THEN** the actor is created, and the third actor gets no `Stopped`, in every store

#### Scenario: A send still needs its target

- **WHEN** a turn deletes an actor and sends it a message without creating it again
- **THEN** the commit is rejected and the message of the turn ends as `failed`

### Requirement: A delete by a turn means what a delete from outside means

A delete by a turn SHALL remove the actor, its mailbox and its descendants, SHALL void a turn that is running on any
of them, and SHALL notify their watchers.

#### Scenario: A turn is running on the target

- **WHEN** a turn deletes an actor while another turn is running on that actor
- **THEN** the running turn commits nothing, and its message ends as `purged`

#### Scenario: A restart while a turn is running on the target

- **WHEN** a turn deletes an actor and creates it again while another turn is running on that actor
- **THEN** the running turn commits nothing, its message ends as `purged`, and the new actor has the state of the
  create and can be claimed

#### Scenario: The target has children

- **WHEN** a turn deletes an actor that has a child
- **THEN** the child is gone too

#### Scenario: A delete by a turn wakes the watcher

- **WHEN** a turn deletes a watched actor, on a node that hosts the watcher, on virtual time
- **THEN** the watcher handles the `Stopped` at once, not after the next poll

#### Scenario: The target is watched

- **WHEN** a turn deletes a watched actor that has a watched child
- **THEN** the watcher of the actor gets a `Stopped` with the cause `deleted`, the watcher of the child gets one with
  the cause `parent`, `by` of both is the actor whose turn deleted them, and the `causedBy` of both is the message of
  that turn

### Requirement: Several removals in one commit give each actor one cause

When one commit removes an actor for more than one reason, an actor that the turn names in a delete SHALL read
`deleted`, the turn's own actor SHALL read `stopped` when it stops, and every other removed actor SHALL read
`parent`. A watcher that the commit removes SHALL get nothing, also when the commit creates it again.

#### Scenario: An actor and its descendant are both deleted

- **WHEN** a turn deletes an actor and also its child, both watched by a third actor
- **THEN** the third actor has one `Stopped` for each, both with the cause `deleted`, in every store

#### Scenario: A turn deletes its child and stops

- **WHEN** a turn deletes one of its two children and stops, and a fourth actor watches all three
- **THEN** it reads `deleted` for the named child, `parent` for the other child and `stopped` for the turn's actor

#### Scenario: A watcher that is restarted in the same commit

- **WHEN** a turn deletes an actor and its watcher, and creates the watcher again
- **THEN** the new watcher has no message and no watch

### Requirement: A turn cannot delete itself or an ancestor

A delete of the turn's own actor SHALL fail at the call. A delete of an ancestor of the turn's own actor SHALL reject
the commit, and SHALL remove nothing.

#### Scenario: Deleting itself

- **WHEN** a handler calls `turn.delete` with the ref of its own actor
- **THEN** the call fails, and the turn can go on and stop itself with `turn.stop()`

#### Scenario: Deleting its parent

- **WHEN** a turn of a child deletes its parent
- **THEN** the message of the turn ends as `failed`, and the parent and the child are still there

#### Scenario: Pausing itself

- **WHEN** a turn pauses its own actor and commits
- **THEN** the turn's message ends as `done`, and the actor's next message waits until it is resumed

### Requirement: A turn may control any actor

A control effect SHALL NOT need the turn's actor to have created its target.

#### Scenario: An actor it did not create

- **WHEN** a turn deletes an actor that was created from outside a turn
- **THEN** the actor is gone

### Requirement: Calls of one turn on the same actor have one result

Asking for the same effect on the same actor twice SHALL apply it once. After a pause and a resume of the same
actor in one turn, the last call SHALL stand. A delete of an actor after a create of the same actor in one turn
SHALL fail at the call. A store that is given a commit that deletes its own actor, or that both pauses and resumes
one actor, SHALL answer `StoreError::Invalid`.

#### Scenario: Pause and resume

- **WHEN** a handler pauses an actor and then resumes the same actor in one turn
- **THEN** both calls succeed, and after the commit the actor is not paused

#### Scenario: The same effect twice

- **WHEN** a handler deletes the same actor twice in one turn
- **THEN** both calls succeed, and the actor is deleted once

#### Scenario: A delete after a create

- **WHEN** a handler creates an actor and then deletes the same actor in one turn
- **THEN** the second call fails; a delete followed by a create is the restart

#### Scenario: A create of a descendant of a deleted actor

- **WHEN** a turn creates an actor that exists as a child of another actor, and deletes that other actor, in either
  order of the two calls
- **THEN** the old child goes with its parent, and the commit creates a new actor at its address

#### Scenario: A malformed commit

- **WHEN** a store is given a commit that deletes its own actor, or one that pauses and resumes one actor
- **THEN** it answers `StoreError::Invalid`, in every store, and applies nothing

### Requirement: A restart on PostgreSQL holds against a create from elsewhere

For an address that one commit both deletes and creates, the result SHALL be the actor of that create, also when the
actor did not exist and another caller creates it at the same moment.

#### Scenario: A restart of an actor that is not there races a create

- **WHEN** a turn deletes and creates an actor that does not exist, while another caller creates the same actor, many
  times
- **THEN** after both calls returned the actor has the state that the turn gave it, every time: in either order of
  the two, the turn's create is the one that stands
