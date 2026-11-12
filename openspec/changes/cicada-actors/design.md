# Design

Described on 2026-11-12 (change loop, Describe), while `cicada-app` is built. The owner handed the decisions of
this change to the main session on 2026-11-11 ("Dor change 5 until 9 i fully trust you"). Every decision below is
made by the main session; each gives the options and why. Reviewed six times, and changed after each review.
"Changed by the reviews" at the end lists what changed.

## Context

What the code does today (release 0.12.0):

- `cicada-extras` holds three things: `DeadLetters`, an actor at `cicada.dead-letters/_` that keeps failed messages
  and can send one again or forget it; `FailureWatcher`, a task that polls the failed outcomes on a timer and feeds
  them to that actor, and skips the failures of the dead-letters actor itself; and `Schedule`, a helper that says
  when a recurring job runs next. The examples and the sample service use all three. Chorus and Karakuri use none
  of them. The spec `outcomes-listing` has a requirement about the failure watcher.
- A watch (ADR 0023, since 0.9.0): an actor that watches another by its address gets a `Failed` when one of its
  messages ends `failed`, and a `Stopped` when it goes away, with the cause and with `by`, the actor whose turn
  removed it. A watch on a type or on `*` tells of failures only. A watch survives the delete and re-create of its
  target. A notification that fails tells nobody. A type may watch only when its family has both `Failed` and
  `Stopped`, with the schemas of the core. Those two schemas are part of the contract of every type that watches,
  and a snapshot test pins them.
- A `Failed` holds the actor, the failed message (id, type, payload, correlation id, sender), the error as a text
  and the attempts. It does not say **why** the message failed.
- A message ends `failed` for different reasons: its handler returned an error or panicked and no retry is left;
  the handler refused it for good (a permanent failure); the turn ran into its maximum duration; the message
  cannot be decoded; the state or the address of the actor cannot be read by its type, or the new state cannot be
  stored; the effects of the turn were refused or its commit was rejected (a send to an actor that is not there, a
  child that exists under another parent); the claim limit (ADR 0025). Only some of these mean that something is
  wrong with the actor. The node knows which it is when it decides that the message fails; the store gets only the
  error text. An observer gets `TurnOutcome::Failed(String)`.
- Control effects (ADR 0024): a turn can delete another actor and create it again in one commit, which is a
  restart: a new state and an empty mailbox. A commit applies its deletes, then its creates, then its sends, and
  writes its notifications last. A delete of an actor that is not there does nothing and writes no `Stopped`. A
  delete after a create of the same address fails at the call, and a buffered effect cannot be taken back.
- A mailbox is read in the order of the time a message is due and then of a number that grows with every message
  of the store. A store reads its clock after it has taken its locks, and a commit that writes into a mailbox locks
  the row of that actor. The clock of a store can stand still: on virtual time, and after a step back.
- A child (ADR 0019) goes when its parent goes.
- A turn creates an actor only through a typed ref. It can send by address with a type name and a JSON payload
  (`send_raw`), which checks at the call that the type takes the message. From outside a turn, `create_actor` lets
  a node that hosts the type refuse an address the type cannot read and a state it cannot decode. Chorus asks for a
  create by address in a turn (its `cicada-requests`, point 8).
- A failed turn commits nothing. A handler cannot test whether its commit will be rejected.
- Changing the contract of a type means a new type name, or stopping every node that hosts it, removing its
  registration and deleting its actors (the operations guide, "Changing a contract").
- Chorus and Karakuri have no type that watches.

ADR 0018: "One crate, replacing `cicada-extras`. A ready-made actor has a fixed type name and stores the messages
it sends for others as an address, a message type name and a JSON payload; a typed helper builds that from a ref
and a message, so a wrong message is still a compile error. First: supervisor, dead letters, topic. Later, each
when someone needs it: schedule (moved from extras), …". It leaves open how a supervisor escalates: "a failed turn
commits nothing, so escalating is a committed turn that sends upward".

## Goals / Non-Goals

**Goals**

- Supervision, dead letters and publish and subscribe as ordinary actors on the public API, with no polling.
- A supervisor that restarts a child only when the child is at fault, and once for one incident.
- `cicada-extras` goes, and nothing a host could use from it is lost.

**Non-Goals**

- Strategies beyond "restart this one child": no one-for-all, no rest-for-one.
- A scheduler actor. The helper `Schedule` moves as it is.
- A guarantee of order across the subscribers of a topic.
- The cause of a failure in the outcome of a message, for HTTP and the command line. It would need a migration.

## Decisions

### 1. A `Failed` says why

The core's `Failed` gets one field:

```rust
pub struct Failed {
    // … the fields of today …
    pub cause: FailureCause,
}

pub enum FailureCause {   // on the wire in kebab-case
    Handler,      // the handler returned an error or panicked, and no retry is left
    Node,         // the node or the store failed around the turn: a read it needed failed, or a panic
                  // outside the handler
    Refused,      // the handler refused the message for good (a permanent failure)
    Timeout,      // the turn ran into its maximum duration
    Message,      // the message cannot be decoded
    State,        // the state or the address of the actor cannot be read by its type
    Commit,       // the effects of the turn were refused, or its commit was rejected
    ClaimLimit,   // the claim limit (ADR 0025)
}
```

- Every `failed` has exactly one cause. The node decides it where it decides that the message fails:

  | Way of failing | Cause |
  | --- | --- |
  | the handler returned an error or panicked, and no retry is left | `handler` |
  | the same, but the store could not schedule the retry | `handler` |
  | the handler returned an error that is not permanent and whose chain of causes holds a `TurnError::Store` or a `StoreError` | `node` |
  | a read of the store the node needed failed, or a panic outside the handler | `node` |
  | the handler refused the message for good | `refused` |
  | the turn ran past its maximum duration, whatever the handler did | `timeout` |
  | the message cannot be decoded | `message` |
  | the state or the address cannot be read by the type | `state` |
  | the handler returned a new state that cannot be stored (it does not encode, or it is `null`) | `handler` |
  | the effects of the turn were refused, or its commit was rejected or refused | `commit` |
  | the claim limit | `claim-limit` |

- The node looks through the whole chain of causes of a handler's error, so a store error that the handler wrapped
  with a context of its own counts too. A permanent error is `refused` whatever it holds: the handler chose to
  refuse.
- `node` keeps a failure of the store away from `handler`. Today a read of the registry that fails after the
  handler returned fails the message, and every `send_raw` and `create_raw` reads the registry: a handler that
  passes such an error on with `?` would otherwise look like a handler that failed, and a supervisor would restart
  a healthy child for a short outage of the store.
- The node decides the cause where it decides that the message fails, and hands it to the store with the commit.
  The store writes it into the `Failed` of each watcher. `TurnOutcome` stays as it is, so an observer is not
  touched.
- `Stopped` does not change.

Why now, and why in the core:

- **A supervisor needs it to be safe.** Without the cause, every failed message would restart a child: a message
  that a client sent wrong, or one that the child refused, would empty its mailbox.
- **Reading the error text is not enough.** The first version of this design read `claim limit:` from the start of
  the error. A refusal by the handler and a failure of the handler have no such mark, and a mark in a text is a
  hint, not a fact.
- **It is cheap now and dear later.** The schema of `Failed` is part of the contract of every type that watches,
  so the change moves each of those types through "Changing a contract". Watches exist since 0.9.0 and the hosts
  have no type that watches. ADR 0023 says the schemas "do not change lightly"; this is their first real user.

Options:

- **A field for the cause (chosen).**
- Hints from the error text. They cannot tell a refusal from a failure.
- A field for the life of the actor as well (the second version of this design: the time the actor was created,
  in both notifications). A supervisor would compare it with a time of its own. The clock of a store can stand
  still, and then two lives have one time. Decision 5 tells lives apart without a clock.

`FailureCause` is a closed set: each value is part of the schema of `Failed`, so a cause added later changes the
contract of every type that watches once more. ADR 0028 says so. Not chosen: a plain text with the known values in
the docs, which a watcher would have to match by hand.

A `Failed` that a node of 0.13.0 wrote has no cause. A watcher of 0.14.0 that still has one in its mailbox cannot
decode it, and that message ends `failed`; a notification that fails tells nobody. The upgrade notes say: let the
watchers handle their notifications before the nodes are stopped.

### 2. The crate, the names, and a stored message

`cicada-actors` replaces `cicada-extras` and uses only the public API of the core.

| Actor | Type name | Key |
| --- | --- | --- |
| Supervisor | `cicada.supervisor` | any name |
| Dead letters | `cicada.dead-letters` | any name; `_` by convention |
| Topic | `cicada.topic` | the name of the topic |

A message that an actor keeps to send later is a `StoredMessage`: `{"type": …, "payload": …}`. A typed helper
builds one from a ref and a message of its family, so a message of the wrong family does not compile:

```rust
let start = StoredMessage::for_ref(&worker, WorkerMsg::Begin { batch: 7 })?;
```

A stored message holds no address, where ADR 0018 says it does: each of the three actors already knows where it
goes (to the child, to the subscribers, to the actor of the failed message), so the address would be stored twice.
ADR 0028 records that.

### 3. The core: a create by address

```rust
pub async fn create_raw(&mut self, address: &ActorAddress, initial_state: Value) -> Result<(), TurnError>;
pub async fn create_child_raw(&mut self, address: &ActorAddress, initial_state: Value) -> Result<(), TurnError>;
```

They are `create` and `create_child` for an address and a JSON state, and follow them in everything else: the
create is idempotent, a `null` state is refused, a child that exists under another parent rejects the commit.

- A call for an address whose type is not registered fails with `TurnError::Invalid`. A read of the registry that
  fails is `TurnError::Store`: that is no verdict on the address.
- A node that hosts the type checks at the call what `create_actor` checks from outside a turn: that the type can
  read the address, and that the state decodes. It refuses with `TurnError::Invalid`.
- A node that does not host the type cannot check either. An address the type cannot read, or a state that no node
  can decode, makes every message of that actor fail with the cause `state` (ADR 0026: JSON that code sends is not
  checked against a schema).

Options:

- **In the core (chosen).** A supervisor cannot be written without it, and Chorus asks for it.
- A starter function per child type, registered with the supervisor type. Chorus does that today, and asks to get
  rid of it.

### 4. The supervisor

A supervisor keeps a list of children. For each it holds the address, the state to create it with, an optional
message to send it after each start, and what it knows of the child: whether a life of it that the supervisor
started is running, the id of the `Restarted` of a restart that is under way, whether it gave up on the child and
why, and the times of its restarts.

```rust
let state = SupervisorState::new()
    .child(Child::new(&worker, &WorkerState::default())?.start(WorkerMsg::Begin { batch: 7 })?)
    .child(Child::new(&mailer, &NoState {})?);
system.create(&supervisor, &state).await?;
system.send(&supervisor, SupervisorMsg::Start).await?;
```

| Message | What the supervisor does |
| --- | --- |
| `Start` | Sends itself one `StartChild` for each child of its state that is not running, with the retry policy below. For every child that is running, it ends a restart under way, and for one that is given up it also clears that mark and its restarts; the child keeps running. A second `Start` starts what the first could not. |
| `StartChild { actor }` | Starts one child: in one commit it creates the child as its own child with the state of its entry, watches it, sends it its start message, and marks it as running. A child that is running is left as it is. |
| `Failed` | A message of a child ended `failed`: decision 5. |
| `Stopped` | A child went away: decision 6. |
| `Restarted { actor }` | Sent by the supervisor to itself in a restart: decision 5. |
| `Escalate { actor, error, restart }` | Sent by the supervisor to itself: decision 7. |

- The children of a supervisor are the children of its state. There is no message to add or remove one while it
  runs: a host that needs another set of children creates another supervisor. The third version of this design
  had `Supervise` for that; with it came children that a restart from above loses, and rules to repair a child
  with it. Nobody asks for it.
- The children are created with `create_child_raw`, so they go when the supervisor goes.
- A supervisor can be a child of another supervisor, with `Start` as its start message. A restart of it builds its
  children again from its entry.
- A notification about an actor that is not one of its children is ignored.
- To take a child away for good, delete it (decision 6). A child that does not exist cannot be taken away that way:
  a delete of an absent actor writes no `Stopped`. Its entry stays, and each `Start`, and each restart from above,
  tries it again (Risks).
- **`StartChild`, `Restarted` and `Escalate` are the supervisor's own.** It acts on them only when it sent them
  itself; from anybody else (a client, a redrive of a dead letter) they end `done` and change nothing. Otherwise a
  send over HTTP, or the redrive of an `Escalate` that dead letters recorded, could make a supervisor above restart
  a tree.
- A node that hosts the type of a child checks its entry when the child starts (decision 3). On another node a
  wrong entry is created, its child fails with the cause `state`, and the supervisor gives up on it (decision 5).
  The docs advise to host a supervisor where its children's types are hosted, so that a wrong entry is found at the
  start.

**One turn for each child at the start.** A turn cannot ask whether an actor exists. A child of the state that
exists already, under another parent or with none, rejects the commit that creates it. In a turn of its own it
fails alone, and the others start.

**A start is tried again.** A `StartChild` is sent with a retry policy: up to 7 attempts, the waits doubling from
one second, 63 seconds in all. A type that is not registered yet, because the node that hosts it is still
starting, is then found on a later attempt. While attempts are left, a call that fails makes the handler fail, so
that the attempt is retried and nobody is told. At the last attempt:

| Case | What happens |
| --- | --- |
| A call is refused: the type of the child is not registered, a hosting node cannot read its address or its state, or its type does not take the start message | The supervisor marks the child as given up, with the error, and escalates without a restart (decision 7). The calls of a start come in a fixed order, the create, then the watch, then the start message; when only the start message was refused, the child was created and runs without it. |
| A read of the store fails | The handler fails; the cause is `node`. Nothing changed, and a later `Start` tries again. |
| The commit is rejected: the actor exists under another parent, or with none | `StartChild` ends `failed` with the cause `commit`. Nothing changed: the child is not running, and a later `Start` tries again. |

### 5. What a supervisor does when a child fails

It looks at the cause.

| Cause | What it means | The supervisor |
| --- | --- | --- |
| `handler`, `timeout` | the child is at fault | restarts the child |
| `state` | the type cannot read the state or the address of the child | gives up on the child, and escalates without a restart above (decision 7) |
| `refused`, `message` | the message is at fault | does nothing |
| `commit` | another actor is missing, or somebody made a mistake | does nothing |
| `node` | the store or the node failed; the child is not at fault | does nothing |
| `claim-limit` | a node died; the message that caused it may be another one (ADR 0025) | does nothing |

A restart is one commit, with its calls in this order: it deletes the child, creates it again with the state of its
entry, sends it its start message, and sends itself `Restarted` for that child. It keeps the id of that `Restarted`
in the entry of the child, and notes the time of the restart. The watch stays. **A restart empties the mailbox of
the child**: the messages in it end `purged`, no dead letter is written for them, and the state of the child is the
one of its entry again.

When a call of a restart is refused, the calls before it stay and the turn commits: when the create is refused, the
child is deleted and not created again; when the start message is refused, the child is created again without it.
Either way the supervisor gives up on the child and escalates without a restart (decision 7). When a read of the
store fails during a restart, the handler fails and nothing is committed: the `Failed` was a notification, which is
not retried, so that restart is lost, and the child keeps running until it fails again.

**One incident is one restart.** While a restart of a child is under way, the supervisor ignores every `Failed`
about it. The `Restarted` whose id the entry holds ends the restart; another one changes nothing. That needs no
clock:

- Everything the old life did was committed before the restart, so its notifications are in the supervisor's
  mailbox before `Restarted`. A child with five failed messages in its backlog is restarted once.
- Everything the new life does is committed after the restart, so its notifications come after `Restarted`.
- One notification of the old life comes after `Restarted`: the `Stopped` of the delete in the restart itself,
  because a commit writes its notifications after its sends. Decision 6 ignores it.

A mailbox is read in the order of the time a message is due and then of a number that grows with every message.
That is the order of the commits as long as the clock of the store does not step back. SQLite does not let its
clock go back. On PostgreSQL a step back of the database's clock between two commits can put a notification on
the wrong side of `Restarted`: one incident may then lead to two restarts, or one failure of the new life may be
missed.

**A `Restarted` that never comes.** An operator who purges or cancels the supervisor's own messages can take a
`Restarted` away. The child then stays in its restart, and its failures are ignored. A `Start` repairs it: it
takes the child back as decision 4 says.

**Why `state` restarts nothing.** It means that the type cannot read the state or the address of the child. A
restart can fix that only when the state went wrong in a turn; mostly it went wrong in a deploy that reads states or
keys more strictly, and then a restart would reset the state of every supervised actor of that type without anybody
choosing it, fail again, reach the limit, and restart the tree above. The key rules of a ref are not part of its
contract (ADR 0021), so such a deploy starts without a word. Giving up keeps the state, and shows it.

Not chosen: a setting for which causes restart. One rule that a host can read in a table is worth more than a
setting; it can come when someone needs another rule.

### 6. A child that goes away

- A `Stopped` with the cause `deleted` whose `by` is the supervisor itself ends a life that the supervisor ended
  itself, in a restart. It is ignored.
- Every other `Stopped` about a child means that the child stopped itself or that somebody else deleted it: the
  child is meant to be gone. The supervisor removes its entry and its watch.
- When such a `Stopped` comes while a restart of the child is under way, the life it is about ended before the
  restart, and the restart has created a new one. The supervisor deletes that new life too. So a child that stops
  itself right after a failure is not brought back, and a delete by an operator holds even while a failure of the
  child waits in the supervisor's mailbox. The new life may have handled its start message before it is deleted.

### 7. The restart limit, and escalation

A supervisor escalates in two ways, and only the first restarts anything above it.

- **Too many restarts.** A supervisor restarts one child at most `maxRestarts` times within `withinSeconds` (3
  times in 60 seconds unless its state says otherwise). When the child fails once more than that, the supervisor
  does not restart it, marks it as given up, and sends itself `Escalate` with `restart: true`. The handler fails as
  a handler does, with the error `restart limit: <actor> failed <n> times in <within>: <error>`, so its cause is
  `handler`: a supervisor above restarts this one, and with it the whole tree, as often as its own limit allows.
- **A child that cannot be started, or whose state cannot be read.** A call of a start or of a restart was refused
  (decisions 4 and 5), or a message of the child failed with the cause `state`: the entry or the type is wrong. A
  restart above would start the child from the same entry and fail the same way, and empty the mailboxes of its
  siblings each time. So the supervisor marks the child as given up and sends itself `Escalate` with
  `restart: false`, whose handler refuses for good, with the error `cannot start <actor>: <error>`, or
  `cannot read <actor>: <error>` for the cause `state`. Its cause
  is `refused`: nobody restarts for it, and dead letters and the watchers of the supervisor see it. A `Start`
  tries such a child again when it is not running, and takes it back when it is.

The step through `Escalate` is needed: the turn that handles the `Failed` of the child must commit (it marks the
child), and a notification that fails tells nobody. This is the "committed turn that sends upward" of ADR 0018.

A child that is given up after too many restarts keeps running, without a supervisor. Pausing it would let its
mailbox grow, and deleting it would throw away what it holds; an operator decides, and a `Start` takes it back. A
supervisor above that restarts this one starts it again from its entry.

Not chosen:

- The supervisor stops itself. Its watchers would get a `Stopped` with the cause `stopped`, which also means "it
  ended in good order".
- A message to a configured address. One more setting, and the watch is the way upward that exists.

### 8. Dead letters

`DeadLetters` keeps failed messages and is told of them by a watch, with no polling.

| Message | What it does |
| --- | --- |
| `Failed` | Records the failed message as a letter. A `Failed` about a message of the dead-letters actor itself is ignored. |
| `Redrive { message_id }` | Sends the failed message again: the same actor, type, payload and correlation id. |
| `Discard { message_id }` | Forgets a letter. |
| `Stopped` | Ignored. |

- A host creates the actor and sets its watch from outside, with `ActorSystem::watch`, `PUT /watches` or
  `cicada create watch --target '*' --watcher cicada.dead-letters/_`. The first version had `Watch` and `Unwatch`
  as messages of the actor; they would repeat what the core has.
- A watch on `*` covers the actor itself. Its own failures (a `Redrive` to an actor that is gone) are no dead
  letters, and the watcher of today skips them too.
- It keeps at most its capacity of letters (1,000 unless the type is built with another); the oldest go first, as
  today.
- **A redrive:** the letter records the id of the new message. A letter that was sent again is not sent a second
  time. When the type of the actor no longer takes the message, the call fails, nothing is sent, and the letter
  records why. When the actor is not there, the commit is rejected: the `Redrive` itself ends `failed` with the
  cause `commit`, and the letter is as it was. When the new message fails too, it is a letter of its own.
- `FailureWatcher` goes, and with it the delay of its timer and the read of the outcomes.

### 9. The topic

A topic sends each published message to its subscribers.

| Message | What it does |
| --- | --- |
| `Subscribe { subscriber }` | Adds the address, and watches the topic itself (a watch that is set already changes nothing). |
| `Unsubscribe { subscriber }` | Removes the address. |
| `Publish { message }` | Sends itself one `Deliver` for each subscriber. |
| `Deliver { subscriber, message }` | Sends the stored message to that one subscriber, with the correlation id of the publish, when it is still a subscriber. |
| `Failed` | About a `Deliver` that the topic sent itself, with the cause `commit`: the subscriber is not there; removes it. Any other is ignored. |
| `Stopped` | Ignored. |

- **One turn for each subscriber (chosen).** A send to an actor that does not exist rejects the commit of its turn.
  With one `Deliver` for each subscriber, a subscriber that is gone fails its own delivery and nobody else's. With
  all sends in the turn of `Publish`, one missing subscriber would fail the publish for everyone.
- **The topic does not watch its subscribers.** A watch on an address tells of every failure of that actor, so the
  topic would run a turn for each failure of each subscriber, and lock its row in their commits. A subscriber that
  is gone, or that never existed, is dropped after its first delivery that is rejected: the topic is told of that
  through the watch on itself.
- **A `Deliver` whose send is refused at the call is refused for good,** with the cause `refused`, sends nothing,
  and keeps the subscriber: whether no type is registered for the address of the subscriber or its type does not
  take the message. The public API does not tell the two apart, and a publisher's mistake must not drop
  subscribers. The failed `Deliver` shows the mistake in the outcomes and to dead letters. An address that never
  gets a type stays until it is unsubscribed; each publish costs it one failed turn.
- **A `Deliver` is tried again when a read of the store fails.** It is sent with the retry policy of a `StartChild`
  (decision 4); a refused send is not retried.
- **A `Deliver` counts only from the topic itself,** and it sends only to a current subscriber: one from anybody
  else ends `done` and sends nothing, and a publish that was queued before an `Unsubscribe` delivers nothing to
  that address after it. A client cannot make a topic send to an address of its choice.
- A failed `Deliver` has the cause `commit`, so a supervisor above a topic would not restart it for that.
- One subscriber gets the messages of a topic in the order they were published, as long as the clock of the store
  does not step back (decision 5), and except for a delivery that was tried again after a failed read of the
  store: a retry is due later, so a later delivery can overtake it.

A publisher builds the message with the typed helper: `TopicMsg::publish(OrderMsg::Shipped { id })?`. The helper
checks that the value is a message of some family; it cannot know the families of the subscribers. Each publish
and each `Deliver` keeps the correlation id of the publish.

### 10. The wire formats

The contracts of the three types are wire formats that hosts depend on. Their JSON is camelCase, as everything
else is. The crate has contract snapshots for the three (`assert_contracts_match`), and sample checks
(`assert_message_sample`, `assert_state_sample`) for what its helpers write.

| Type | Messages | State |
| --- | --- | --- |
| `cicada.supervisor` | `Start {}`, `StartChild { actor }`, `Restarted { actor }`, `Escalate { actor, error, restart }`, `Failed`, `Stopped` | `{ children: { <address>: { state, start?, running, restartId?, givenUp, lastError?, restarts: [time] } }, maxRestarts, withinSeconds }` |
| `cicada.dead-letters` | `Redrive { messageId }`, `Discard { messageId }`, `Failed`, `Stopped` | `{ letters: { <message id>: { actor, messageType, payload, correlationId, from, error, cause, attempts, recordedAt, redrivenAs?, redriveError? } } }` |
| `cicada.topic` | `Subscribe { subscriber }`, `Unsubscribe { subscriber }`, `Publish { message }`, `Deliver { subscriber, message }`, `Failed`, `Stopped` | `{ subscribers: [address] }` |

`start` and `message` are stored messages, `{ type, payload }` (decision 2).

### 11. `Schedule` moves as it is

`cicada_extras::Schedule` becomes `cicada_actors::Schedule`, unchanged: a helper that says when a recurring job runs
next. ADR 0018 lists a schedule under "later"; that is an actor, and it stays later. Not chosen: moving the helper
into the examples, where a host that uses it would lose it.

### 12. The examples and the sample service

The supervision example is built on the new actors: dead letters with a watch on `*`, and a supervisor with a
child that fails. The sample service hosts the three types, creates `cicada.dead-letters/_` and its watch in
`on_start`, and loses `FailureWatcher` and `FAILURE_WATCHER_INTERVAL` (also in the end-to-end compose file).

## Risks / Trade-offs

- **`Failed` changes** (decision 1). Every type that watches has a new contract. In this repository those are test
  types and the three new actors; a host with a type that watches follows "Changing a contract".
- **A restart empties the mailbox of the child, writes no dead letter for what it purges, and gives the child the
  state of its entry.** That is what a restart is (ADR 0024); the docs of the supervisor say so first.
- **What may be supervised.** A supervisor is for actors whose state can be built again from the state of their
  entry. Dead letters and a topic are not such actors: a restart would lose their letters and their subscribers,
  and a watch that was set for them from outside. A supervisor may be supervised. The docs say so.
- **Where a supervisor runs.** On a node that does not host the types of its children, a wrong entry is caught only
  when the child first fails with `state`; the supervisor then gives up on it (decisions 4 and 5).
- A child whose type is not registered within 63 seconds of its start is given up; a later `Start` tries it again
  (decision 4).
- A child whose state its type cannot read is given up, not restarted, also when a restart would have helped
  (decision 5).
- A child that cannot exist (its type was removed, or its entry is wrong) cannot be taken away while its
  supervisor lives: each `Start`, and each restart from above, tries it for 63 seconds and escalates
  `cannot start` again (decision 4).
- A child that stopped itself comes back when a supervisor above restarts its supervisor: the entry of the
  supervisor above still holds it.
- A new value of `FailureCause` changes the contract of every type that watches (decision 1).
- A short outage of the store while a supervisor handles a failure loses that restart (decision 5).
- The children of a supervisor are fixed by its state (decision 4).
- The rule of decision 5 is fixed. A host whose child should start over when it refuses a message has no setting
  for it.
- A supervisor is one actor: it handles the failures of its children one after the other.
- The state of a child and its start message are stored as JSON in the supervisor. When the type of the child
  changes so that they no longer fit, a restart deletes the child and cannot create it again, and the supervisor
  gives up on it (decision 5).
- On PostgreSQL a step back of the database's clock can turn one incident into two restarts (decision 5).
- An operator who purges a supervisor's own mailbox can leave a child in its restart until the next `Start`
  (decision 5).
- A child that stops itself right after a failure may handle its start message once more before the supervisor
  deletes it (decision 6).
- A state or an address from `create_raw` that the type cannot read (decision 3).
- A topic sends itself one message for each subscriber and each publish; a topic with many subscribers is slow. It
  drops a subscriber that is gone only at its next delivery, and keeps an address that has no type, with one failed
  `Deliver` for each publish (decision 9). A delivery that is tried again can come after a later one.
- A letter, a `Deliver` and a child entry each wrap a value some levels deeper. The proposal of
  `value-depth-limit` has a line on it.

## Migration Plan

Version 0.14.0. No migration of the store. Not a rolling deploy for a system that has a type that watches, or dead
letters.

1. Replace `cicada-extras` by `cicada-actors`. `Schedule` keeps its name. Drop `FailureWatcher`.
2. A type that watches: its contract changes with `Failed`. Let it handle the notifications in its mailbox, then
   follow "Changing a contract" for it. A handler that builds a `Failed` by hand adds the cause.
3. Dead letters: its contract changes too, and its letters are lost.
   1. Save the letters if they matter: `cicada get actor cicada.dead-letters/_ -o json`.
   2. Stop every node. While a node of 0.13.0 runs, its `FailureWatcher` creates the actor again after it is
      deleted.
   3. Start one node of 0.14.0 that does not host the type (`serve --types none`). Delete the actor and then the
      registration: `cicada delete actor cicada.dead-letters/_`, `cicada delete type cicada.dead-letters`. A node
      that hosts the type refuses to start until that is done.
   4. Start the other nodes, create the actor again and set its watch on `*`.
4. A supervisor and a topic are new; nothing moves.

## Changed by the reviews

- **First review.** The first version restarted a child for every `Failed` but one whose error started with
  `claim limit:`. The review showed that a refusal or a message that cannot be decoded would empty a child's
  mailbox, that a child with a backlog would be restarted once for each queued failure, and that one mistaken
  `Supervise` under a supervisor above would restart a whole tree. Decision 1 (the cause) is the answer to the
  first. `OnFailure` and the `Watch` and `Unwatch` of dead letters went; the topic watches itself; the wire
  formats are written down.
- **Second review.** The second version told lives apart by the time an actor was created, in both notifications.
  The review showed that a store's clock can stand still, so that two lives share a time. Decision 5 now uses a
  message of the supervisor to itself and the order of its mailbox, and the field for the life is gone again. A
  `Supervise` that cannot be carried out is refused and no longer escalates; `Remove` is back, because deleting a
  child directly is not reliable; a restart whose create is refused deletes the child, and the design says so;
  every failure has a cause; dead letters skip their own failures; the spec `outcomes-listing` loses its
  requirement about the failure watcher; the steps for dead letters in the migration plan can be carried out.
- **Third review.** The second version escalated with a restart when a child of the state could not be started; the
  supervisor above would start it from the same entry again, three times, and empty the mailboxes of the siblings
  each time, up to the root. Such an escalation now restarts nothing (decision 7). `Start` starts each child in a
  turn of its own, so that one child that exists elsewhere does not block the others; the calls of a start and of a
  restart have a fixed order; a `Stopped` that the supervisor did not cause removes a child also while it is
  restarting, so a delete is reliable and `Remove` went again; `Supervise` repairs a child that is given up or stuck
  in a restart; a cause `node` keeps failures of the store from restarting a child; the topic no longer watches its
  subscribers; what a restart loses, and which actors may be supervised, are named.
- **Fourth review.** An edit of the main session had cut decisions 3 to 10 out of this file; they are back. A failed
  read of the store that a handler passes on is `node` now, so an outage restarts no healthy child; the table in
  decision 1 gives the cause of every way of failing. `Supervise` went: the children of a supervisor are those of
  its state. A start is tried again for about half a minute, so that a type whose node starts later is found. A restart
  is ended only by its own `Restarted`. A supervisor is to run where its children's types are hosted. The topic no
  longer tells an address without a type from a type that does not take the message, which the public API cannot
  do, and keeps the subscriber in both cases.
- **Fifth review.** `state` gives up on a child instead of restarting it: a deploy that reads states or keys more
  strictly would otherwise reset every supervised actor of the type and restart the tree above. A `Start` takes back
  a child that is given up and still running. The supervisor's own messages count only from itself. A refused
  delivery of a topic fails for good, so that it is seen, and a delivery is tried again when the store fails. The
  rule for `node` names the whole chain of causes. The retries of a start add up to 63 seconds. The steps for dead
  letters in the migration plan stop every node first.
- **Sixth review.** A `Start` ends a stuck restart of any running child, which the fifth version promised but did not
  do. A new state that cannot be stored is a fault of the handler (`handler`), not `state`. A `Deliver` counts only
  from the topic itself and sends only to a current subscriber, and its retry is an exception to the order. What
  cannot be removed, what comes back, and that the set of causes is closed, are named.

## Open Questions

None.
