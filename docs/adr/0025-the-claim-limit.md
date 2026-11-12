# 0025. The claim limit

Date: 2026-11-12

## Status

Accepted

## Context

ADR 0018 replaces the plugin API with one observer that has no say in a turn. `CrashLoopGuard` was a plugin that
refused a turn, so it goes, and ADR 0018 leaves the answer to the change `turn-observer`: "a claim limit that fails
the message". Without one, a message that kills its node is claimed again for ever.

The guard counted every claim of a message that did not end in a commit, also a claim that a node gave back when it
stopped. That was acceptable for an opt-in guard with a warning in its docs. It is not for a limit in the core.

The limit ends a message that no handler finished, which changes the wording of G4, and the count behind it is new
in the wire format. The project rule wants both recorded.

## Decision

- **The core has a claim limit, on by default.** `NodeConfig::max_lost_claims` is 5 unless it is set; `None` turns
  it off. It is one number per node. There is no limit per type or per message.
- **It counts lost claims.** A claim is lost when another claim takes its place although its turn neither committed
  nor gave the lease back: its node died, its lease ran out, or its commit could not get through. A turn whose
  lease ran out can still commit while no other claim came.
- **A message counts its lost claims, and the claim writes the count.** `lostClaims` is a new count on a message.
  A claim of an actor that still holds a lease that nobody committed or gave back adds 1 to the count of the
  message of that lease. The count never goes back to 0. A release clears the lease, so a node that stops in good
  order and gives its claims back loses none: that says nothing about the message. `attempt` stays what it is: it
  counts every claim, and `attempt > 1` still
  tells a handler that an earlier run may have happened.
- **The limit fails the message through a commit.** A node that claims a message with as many lost claims as its
  limit does not decode or run it. It commits the message as `failed` with the error
  `claim limit: <lost> lost claims`. The retry policy of the message is not asked. Because it is a commit, the
  watchers of the actor are told (ADR 0023). The observers are told after that commit.
- **G4 names the limit.** Handler execution is at-least-once: a handler runs again only when its turn ended without
  a commit. A message whose claims were lost as often as the limit allows ends `failed`, whether or not its handler
  ever ran.

## Consequences

- A node that had no guard now has a limit. A healthy message can reach it: five turns of one message that each
  lose their lease to a long pause or a partition. Such a message is in the failed outcomes and can be sent again.
- The limit cannot tell the message that kills a node from the messages that ran beside it. A node that dies loses
  the claim of every turn it runs, so a message that kills its node at once, as often as the limit allows, takes
  the messages the node claimed with it each time to the limit too. Without a limit the same messages would be
  claimed and lost for ever. Running a message that has lost claims alone on its node would find the cause exactly;
  that is left to a later change.
- A node that stops in good order but leaves a lease to run out (a handler that ended, or one that does not yield)
  loses that claim, and so does a node that is killed hard as a matter of routine; only a release is not counted.
- The schema gets an appended migration in both stores: the column `lost_claims` of a message, 0 for every message
  that exists, so no message starts with lost claims. It is not additive: a node of the release before that keeps
  running counts no lost claim, and on PostgreSQL its prepared statements fail on the changed table. Every old
  node is stopped first.
- The wire format grows: `lostClaims` in the view of a message.
- The statements of the PostgreSQL store list their columns from this release on, so that a later migration that
  only adds a column can be additive again.
- `Message` gets a field. Code that builds one by hand adds it.
- Nodes with different limits each apply their own. A system sets one value on all its nodes.
- A watcher can tell a failure by the limit from a failure of a handler by the start of the error, `claim limit:`.
  It is a hint: a handler can return an error with the same start.
