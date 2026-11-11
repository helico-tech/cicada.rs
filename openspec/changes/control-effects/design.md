# Design

Described with the owner on 2026-11-11 (change loop, Describe), five changes ahead of the build at the owner's
request. Every decision below is made by the owner unless it says otherwise. Brought up to date on 2026-11-11 with
the code of 0.8.0 and the design of `watches`, which is built before this change. What the main session decided
then is marked "its turn", and what it decided after the review "first review"; the owner left the open points of
this change to it ("you have my approval to decide for me"). Reviewed twice; what the main session decided after
the second review is marked "second review".

## Context

What the code does today (release 0.8.0), and what `watches` alters:

- A turn buffers four kinds of effect: creates, sends, cancels of its own pending messages, and its own stop. A
  commit applies them in this order: the outcome, the cancels, the creates, the sends, the state, the release of
  the lease.
- Pause, resume, purge and delete of an actor exist as calls from outside a turn (`set_paused`, `purge`, `delete`),
  each its own transaction:
  - **pause:** messages still queue up, no new turn is claimed, a running turn finishes;
  - **purge:** every pending message ends `purged`, except one that is being handled; the actor stays;
  - **delete:** the actor and its mailbox go; every pending message ends `purged`, the one being handled too. A lease
    is voided, so a turn that runs on the actor commits nothing.
  On an actor that does not exist they answer "not there" and change nothing.
- A delete also removes every descendant (`child-actors`), and an actor knows its parent. After `watches`, a
  removal puts a `Stopped` into the mailbox of each actor that watches the removed actor by its address, with a
  cause and with `by`, the actor whose turn removed it. A commit then also sets and removes the watches of its
  turn, before its other effects, and carries the id of its event, from which the store names its notifications.
- The randomised comparison of the in-memory store with SQLite (`tests/fuzz`) builds commits with creates, child
  creates and, after `watches`, watches. The simulation of nodes under faults checks G1 and G3 to G6.
- A turn names another actor by a typed ref (`send`, `create`) or by an address (`send_to`, `send_raw`).
- A delete from outside a turn already lets two handlers run for one address for a moment: the delete voids the
  lease, the actor can be created and claimed again, and the old handler runs on until its next heartbeat fails or
  its blocking code returns. It commits nothing. G1 does not say so.
- A resume from outside a turn wakes the nodes; on PostgreSQL the store announces it.
- On PostgreSQL a commit locks the committing actor together with every actor it sends to or creates, in address
  order, which avoids deadlocks between commits. The removal of a tree locks level by level and can deadlock; the
  store re-runs it.
- A send to an actor that does not exist rejects the commit, and the message ends `failed`.
- ADR 0018 decides: a turn can pause, resume, purge and delete another actor, atomically with its other effects; no
  effect writes another actor's state; to restart an actor a turn deletes it and creates it again, and in one
  commit deletes are applied before creates.

## Goals / Non-Goals

Goals:

- An actor that manages others can pause, resume, purge, delete and restart one in the same commit as its own
  state, or not at all.
- Each effect means what the call from outside a turn means today. There is one "delete", whoever does it.
- A controlling turn does not fail because the actor it controls went away a moment earlier.

Non-Goals:

- Writing or reading another actor's state.
- Cancelling another actor's pending messages or its running turn.
- A create by address and JSON state (`cicada-actors` says why a supervisor may need one).
- Telling the controlling turn whether the actor existed.

## Decisions

### 1. A delete by a turn voids a running turn, as a delete from outside does

When a turn deletes an actor on which another turn is running, that turn loses its lease and commits nothing, and
its message ends `purged`. What its handler already did outside the store stays done.

Not chosen: a delete that waits, by failing the deleting commit while a turn runs there (a supervisor could not
remove an actor that is stuck).

### 2. A control effect on an actor that does not exist does nothing

All four effects mean "make it so". An actor can stop itself at any moment, so a controlling turn cannot know that
its target is still there, and it must not fail for that. A send is different: a send that goes nowhere loses a
message, so it still rejects the commit. A restart works whether or not the actor is still there:

```rust
turn.delete(&Worker { id: "w1".into() })?;
turn.create(&Worker { id: "w1".into() }, &WorkerState::default())?;
```

Not chosen: failing the commit as a send does; failing it for pause, resume and purge only.

### 3. A turn cannot delete itself or an ancestor

`turn.delete` of the turn's own actor fails at the call; the way to go is `turn.stop()`. A delete of an ancestor
would remove the tree that holds the deleting actor. Only the store knows the ancestors, so it rejects that commit,
and the message ends `failed`. Pause, resume and purge of the turn's own actor are allowed and need no rule of their
own: a purge leaves the message that is being handled, as always.

Not chosen: allowing both (nothing that is planned needs it: a supervisor escalates by a message upward).

### 4. A turn may control any actor

There is no check of ownership. Actors are the trusted code of one system and can already send anything to anyone.
A supervisor manages actors it did not create.

Not chosen: only its own descendants.

### 5. The order of effects in one commit

Decided by the main session; ADR 0018 fixes "deletes before creates".

A commit applies: the watches of the turn (`watches`), the outcome, the cancels, the deletes, the creates, the
purges, the pauses and resumes, the sends, the state, the notifications of `watches`, and the release of the lease.

- **Watches before deletes** (its turn): a turn that watches an actor and deletes it in the same commit is told of
  it.
- **Deletes before creates** gives the restart of decision 2.
- **Purges before sends** lets a turn empty a mailbox and put a first message into it, in one commit.
- **Creates before pauses** lets a turn create an actor that starts paused.

Effects of one commit that name the same actor:

| The turn asks for | What happens |
| --- | --- |
| delete, then create | a restart: the old actor and its mailbox go, a new one is created |
| delete, then `create_child` | the same, and the new actor is a child of the turn's actor |
| delete and a send, without a create | the send meets no actor, so the commit is rejected, as today |
| delete and a purge, a pause or a resume, without a create | the delete is applied; the others meet no actor and do nothing |
| create, then delete | the delete fails at the call (`TurnError::Invalid`): a restart is written as delete, then create (first review) |
| pause and resume | the last call stands (second review), as for `watch` and `unwatch` |
| any effect twice | applied once |

A delete after a create of the same address is refused because deletes are applied first: without the rule, a turn
that creates an actor and then deletes it would end with the actor there. So delete, create, delete fails at the
third call.

The order of the calls does not matter otherwise. One case reads oddly and is as it should be (second review): a
turn that creates Y and deletes X, where Y is a descendant of X, ends with a new Y, in whatever order it made the
two calls. The old Y goes with the tree of X, and the create makes a new one.

**What a store answers to a commit that a turn would not build** (second review). The checks above are made by the
turn. A store is also called without one, by the tests and by the randomised comparison, so it has its own check,
as for the creates of `child-actors`: a commit that deletes its own actor, or that both pauses and resumes one
actor, is a malformed request and answers `StoreError::Invalid`, in every store, before the store is read. The check
is one shared function in `store::support`.

### 6. What the calls look like

Decided by the main session; written out after the first review.

Each effect has two calls, as a send has `send` for a ref and `send_to` for an address. An address is needed because
a supervisor gets the address of its target from a notification, not a ref.

```rust
pub fn pause<R: ActorRef>(&mut self, actor: &R) -> Result<(), TurnError>;
pub fn pause_at(&mut self, actor: &ActorAddress) -> Result<(), TurnError>;
// and the same pair for resume, purge and delete
```

Each call buffers one effect and returns no result about the target: whether it existed is decided at the commit.
A call fails with `TurnError::Invalid`, and buffers nothing, for a ref that has no address, for a delete of the
turn's own actor, and for a delete after a create of the same address.

### 7. What each effect does, in the words of today's calls

Decided by the main session; it follows from the proposal.

- **pause:** no new turn is claimed for the actor; messages queue up; a running turn finishes.
- **resume:** turns are claimed again.
- **purge:** every pending message of the actor ends `purged`, except one that is being handled; the actor stays.
- **delete:** the actor, its mailbox and its descendants go (`child-actors`). Each removed actor that is watched by
  its address notifies its watchers with `Stopped` (`watches`): the cause is `deleted` for the actor itself and
  `parent` for a descendant, `by` is the actor whose turn deleted it, and the `causedBy` of the notification is the
  message of that turn. So a supervisor tells its own restart from an operator's delete, which has no `by`. The
  schema of `Stopped` does not change.

### 8. PostgreSQL locks the controlled actors with the others, in address order

Decided by the main session.

A commit already locks its own actor and every actor it sends to or creates, in address order, and after `watches`
the watchers of its own actor when it fails or stops. The actors it pauses, resumes, purges or deletes join that
list, and so do the watchers of each actor it deletes, which the commit reads first and reads again after the
locks, as `watches` describes (its turn). The tree below a deleted actor is locked after that, level by level, as
`child-actors` does for every removal; that part can deadlock with another commit and is re-run, as today.

Two more rules for PostgreSQL (first review):

- **A restart of an actor that is not there.** A row that does not exist cannot be locked. If another transaction
  created the actor between the lock and the create, the delete would see nothing and the create would leave that
  other actor in place: the commit would succeed with a state that no order of the two gives. So for an address
  that a commit both deletes and creates, the store first makes sure that it holds the lock of a row at that
  address, as a create already does, and only then removes what is there and creates the new actor.
- **A restart of an actor that is not there removes nothing** (second review). The row that the store inserts to
  hold the lock is its own. Nothing was removed, so no `Stopped` goes to the watchers of that address, in any
  store.
- **A resume by a commit wakes the nodes**, as a resume from outside does: the store announces it.

And one rule for the node, on every store (second review): it wakes its own loop after a commit with a control
effect, as it does for a send. A resume makes messages due, and a delete of a watched actor writes notifications;
without the wake-up both would wait for the next poll on SQLite and in memory.

### 9. A restart keeps a child a child only when its parent does it

Decided by the main session (its turn); it is decision 9 of `child-actors`.

A parent restarts its child with a delete and a `create_child`. When another actor restarts it, with a delete and a
create, the subtree of the child is gone and the new actor has no parent; the next `create_child` of it by the old
parent is then rejected, as for every actor that exists with another parent or none. The docs of the restart say
so.

Not chosen: a parent address on a create (`child-actors`, decision 1).

### 10. The randomised comparison of the stores builds control effects

Decided by the main session (its turn), as for `child-actors` and `watches`.

The order of decision 5 and the table below it are written three times, once per store. The comparison of
`tests/fuzz` gets the four effects in its generated commits, on addresses of its pool, so that deletes meet
creates, sends, watches and trees in every mix.

The simulation of nodes under faults gets a control effect as well (first review): one of its actors restarts
another in some of its turns, and the checks of G3 and G4 hold for both.

The conformance suite gets two cases for PostgreSQL: two actors that delete and create each other's neighbours
while a third sends to them, and a restart of an actor that is not there while another caller creates it. Each runs
many times; every call ends, and after the second the actor has the state of the restart.

### 11. Several removals in one commit

Decided by the main session (first review).

A turn may delete an actor and one of its descendants, or delete its child and stop. Then one commit removes one
actor for two reasons, and the cause that its watchers read must not depend on which removal a store runs first.

- An actor that the turn names in a delete reads `deleted`.
- The turn's own actor, when it stops, reads `stopped`.
- Every other removed actor reads `parent`.
- `by` is the turn's actor for all of them.
- A watcher that the commit removes gets nothing, also when the same commit creates it again: the new actor is
  another life, with no watches.

### 12. An ADR records the rules and the new wording of the guarantees

Decided by the main session (first review). ADR 0018 says that a turn can pause, resume, purge and delete another
actor, and that deletes come before creates. It leaves the running turn "to the changes" and holds none of
decisions 2 to 5. `docs/adr/0024-control-effects.md` records them, with the wording of the guarantees that change:

- **G1** says what an instance is: one life of an address, from its create to its removal. A handler of an earlier
  life may still run for a moment after the actor was deleted and created again; it commits nothing.
- **G2** names the control of other actors among the parts of a commit.
- **S7** says that a commit can end the pending messages of another actor, by a purge or a delete.
- S2 keeps its wording: "all of its parts or none" covers the new parts.

The main session writes the ADR with this design and commits it with the change when it is ready.

### 13. The node logs each control effect

Decided by the main session (first review). Nothing records who paused, purged or deleted an actor that nobody
watches. The node writes one `tracing` event at the info level for each control effect of a turn whose commit it
saw succeed, with the acting actor, the effect and the target. It does not say whether the target existed: the
commit does not report that.

This is a log line and not a requirement of the spec (second review): no test in the repository reads `tracing`
events, and a node whose commit was applied but not acknowledged does not know it and logs nothing.

Not chosen: a record in the store (a table for something a log line does).

### 14. How this fits ADR 0015

Decided by the main session (second review).

ADR 0015 refused to take a lease away from a running turn, because a second handler could then run for the same
message. `docs/architecture.md` says: "A lease is never taken away from a running node: there is no operation that
does so". A delete has always ended the lease with the actor, and a restart by a turn makes that common. It is not
what ADR 0015 refused: the message of the turn that loses its actor ends `purged` and is never handled again, so
no message is handled twice (G6), and the actor that is created afterwards is another life (G1, decision 12). ADR
0024 says so, and the sentence in `docs/architecture.md` is made exact: a lease ends only with its turn or with
its actor.

## Risks / Trade-offs

- **A mistyped address in a control effect is not noticed** (decision 2). → A typed ref makes that unlikely; the
  docs say that an effect on an actor that is not there does nothing.
- **Any actor can delete any other** (decision 4). → As any actor can send anything to any other. The docs say so.
- **A delete by a turn discards a turn that is running on the target**, and what that handler did outside the store
  stays done. → The same as a delete from outside; the docs say so where they describe `turn.delete`.
- **More rows locked per commit on PostgreSQL.** → In address order, so commits do not deadlock each other; only the
  removal of a tree can, and it is re-run.
- **A large tree removed by a turn is a large commit of that turn.** → As for `turn.stop()` after `child-actors`.
- **A restart by another actor than the parent makes a child independent** (decision 9). → The docs say that a
  parent restarts its own children, with `create_child`.
- **After a restart two handlers can run for one address for a moment** (first review): the old one until its next
  heartbeat fails or its blocking code returns. It commits nothing, but what it does outside the store overlaps
  with the new one. → That is so today for a delete from outside; a restart by a turn makes it common. G1 says so
  now (decision 12), and the docs of the restart do.

## Migration Plan

No schema change and no contract change: the effects are applied by the commit of the node that runs the turn, and
an actor on a node of the previous version is paused or deleted as it would be from outside. Nodes before and after
can share a database.

The guarantee G2 gets new parts, which the README counts as a breaking change: 0.10.0 after 0.9.0.
