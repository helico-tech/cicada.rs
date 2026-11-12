# 0028. The ready-made actors

Date: 2026-11-12

## Status

Accepted. It changes the schema of `Failed` that ADR 0023 fixed, and the rule of ADR 0018 that a stored message
holds an address. It outdates the consequence of ADR 0025 that a watcher tells a failure by the claim limit only by
the start of its error: the cause says it.

## Context

ADR 0018 adds a crate of ready-made actors, first a supervisor, dead letters and a topic, on the public API of the
core. It leaves open how a supervisor escalates: "a failed turn commits nothing, so escalating is a committed turn
that sends upward". The change `cicada-actors` settles that, and finds two things the core lacks for a supervisor.

A `Failed` (ADR 0023) holds the error of a failed message as a text. It does not say why the message failed, and a
message fails for reasons that have nothing to do with its actor: a client sent it wrong, the actor refused it,
another actor was missing, or a node died (ADR 0025). A supervisor that restarts a child for each of those empties
the mailbox of a healthy actor. The failures a child had queued before a restart look like failures of its new
life. And a turn can create an actor only through a typed ref, while a supervisor does not know the types of its
children when it is compiled.

ADR 0023 says the schemas of the two notifications "do not change lightly", because they are part of the contract
of every type that watches. Watches exist since 0.9.0, and the supervisor is their first real user.

## Decision

- **A `Failed` holds a cause:** `handler`, `node`, `refused`, `timeout`, `message`, `state`, `commit` or
  `claim-limit`. Every failed message has exactly one. A failed read of the store is `node`, also when a handler
  passes it on, so that an outage of the store restarts no healthy child. `Stopped` does not change.
- **A supervisor tells the lives of a child apart by the order of its own mailbox, not by a clock.** A restart
  sends the supervisor a message to itself. What the old life did is in the mailbox before that message, and what
  the new life does comes after it. A field for the time an actor was created was considered and dropped: the
  clock of a store can stand still.
- **A turn can create an actor by its address,** with a state given as JSON: `create_raw` and `create_child_raw`.
  A node that hosts the type refuses an address or a state it cannot read; another node cannot check them.
- **A supervisor restarts the one child that is at fault,** for the causes `handler` and `timeout`, in one
  commit: it deletes the child, creates it again with the state it holds for it, and sends it its start message.
  It does nothing for the other causes, and one incident is one restart. There is no strategy for a group.
- **A supervisor escalates by failing a message of its own,** in two ways. After too many restarts of one child it
  marks the child as given up, in a turn that commits, and sends itself a message whose handling fails as a handler
  does: the watchers of the supervisor are told with a `Failed` of the cause `handler`, and a supervisor above
  restarts it with its whole tree. When a child cannot be started at all, because its entry is wrong, the message
  is refused for good instead: a restart above would fail the same way, so nobody restarts for it, and it is still
  seen. A notification that fails tells nobody, and a supervisor that stops itself would look like one that ended
  in good order.
- **A `Stopped` that the supervisor did not cause removes a child,** also while a restart of it is under way: a
  child that stops itself, or that an operator deletes, stays gone.
- **A stored message is a type name and a payload, without an address,** where ADR 0018 says it holds one: each of
  the three actors knows where its stored messages go.
- **A topic delivers to each subscriber in a turn of its own,** so that a subscriber that is gone fails its own
  delivery and nobody else's. It does not watch its subscribers: that would bring it every failure of each.
- **The children of a supervisor are fixed by its state.** It starts each in a turn of its own and tries a start
  again for 63 seconds. A child whose state or address its type cannot read (the cause `state`) is given up and
  shown, not restarted: a restart would reset it after a deploy that reads more strictly, and fail again.
- **The names `cicada.supervisor`, `cicada.dead-letters` and `cicada.topic` are the type names of these actors.**

## Consequences

- Every type that watches gets a new contract with this release, and goes through "Changing a contract". A
  `Failed` of the release before that is still in a mailbox cannot be decoded by a watcher of this release.
- An observer is not touched: `TurnOutcome` does not carry the cause.
- A restart empties the mailbox of a child, writes no dead letter for what it purges, and gives the child a new
  state. A supervisor is for actors whose state can be built again; dead letters and a topic are not.
- The rule for which causes restart is fixed.
- A state from a create by address that no node can decode makes every message of that actor fail with the cause
  `state`; a supervisor then gives up on it and shows it.
- `FailureCause` is a closed set. A cause added later changes the contract of every type that watches again.
- Dead letters are told of a failure in the transaction of the failure. The polling watcher is gone.
- The contracts of the three actors are wire formats that hosts depend on.
