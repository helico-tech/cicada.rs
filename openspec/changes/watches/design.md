# Design

Described with the owner on 2026-11-11 (change loop, Describe), four changes ahead of the build at the owner's
request. Every decision below is made by the owner unless it says otherwise. Brought up to date on 2026-11-11 with
the code of 0.7.0 and the design of `child-actors`, which is built before this change. What the main session
decided then is marked "its turn", and what it decided after the review "first review"; the owner left the open
points of this change to it ("you have my approval to decide for me"). Reviewed three times; what the main
session decided after the second and the third review is marked "second review" and "third review".

## Context

What the code does today (release 0.7.0), and what `child-actors` alters:

- A message gets the outcome `failed` in one place only: a commit with `TurnOutcome::Failed`. That covers a handler
  that failed for good, a panic, a timeout, a state that cannot be decoded or encoded, a commit that was rejected,
  and a message that cannot be decoded. An attempt that is retried gets no outcome
  (`Rescheduled`), so it is not a failure in this sense.
- An actor goes away in two places: a commit with `StateChange::Stop` (`turn.stop()`), and `delete` from outside a
  turn. After `child-actors`, both also remove every descendant, in the same transaction.
- The stores are inside `crates/cicada`, behind the private trait `Backend`. Their conformance cases and the
  randomised comparison of the in-memory store with SQLite (`tests/fuzz`) are tests of the core.
- A commit on PostgreSQL locks every actor row it touches with one statement, in address order: its own row, its
  creates and the targets of its sends. So two commits cannot deadlock. The removal of a tree (`child-actors`) locks
  level by level and can; a transaction that PostgreSQL aborts for a deadlock is run again.
- A send in a commit to an actor that does not exist rejects the commit.
- A retry of a failed message from outside (`retry_message`) sends a new message with the old one as its
  `causedBy`. A retry by filter (`bulk_retry`) skips a failed message that has such a child: it counts as retried.
- A node wakes its dispatch loop for the sends of its own commits; PostgreSQL also announces a send to the other
  nodes. A delete from outside wakes nobody, because it writes no message.
- The simulation of nodes under faults (`tests/fuzz`, the checks of G1 to G6) has no watcher.
- No store draws a message id today: an id comes from the sender or from the runtime.
- The schema is at version 3, and `child-actors` adds version 4. A node checks the schema version only when it
  opens its store; one that is already running does not notice a later migration.
- `FailureWatcher` in `cicada-extras` polls the outcomes under an address prefix on a timer and sends each failure to
  a dead-letters actor. With the empty prefix it sees every failure of the system. `cicada-actors` replaces it with
  a watch.
- ADR 0018 decides: a watch is a stored pair of a target and a watcher; the target is one actor, one type, or a
  dotted type prefix at a segment boundary; a notification is a core message with a fixed type name and schema,
  `Failed` (message id, error, attempts) or `Stopped`; a missing watcher is skipped and its watch removed; a watch
  survives delete and re-create of its target; deleting a watcher removes its watches; a notification that fails
  notifies nobody; a successful message notifies nobody; the watched actor does not know its watchers.

## Goals / Non-Goals

Goals:

- An actor learns that a message to a watched actor failed, or that a watched actor went away, in the transaction
  of that event and without polling.
- A notification is an ordinary message: one handler, one contract, the tools that exist.
- A watch can never make the commit of the watched actor fail, and two watchers can never loop.

Non-Goals:

- A filter on notifications, or notifications for `done`, `cancelled` or `purged` messages.
- A watch that covers only the messages its watcher sent.
- Crash loops; they produce no `failed` outcome until `turn-observer`.
- The supervisor and dead letters built on watches (`cicada-actors`).

## Decisions

### 1. A watcher takes a notification as a message of its own family


The core has two public types, `Failed` and `Stopped`, each with a JSON Schema. A watcher declares them as variants
of its message family, under exactly those type names. A notification is then a message like any other: it is in
the contract, the mailbox, the outcomes and the command line, and the one handler handles it.

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum SupervisorMsg {
    Failed(cicada::Failed),
    Stopped(cicada::Stopped),
    Restart { name: String },
}
```

Setting a watch is refused when the watcher's type does not accept these message types. A family that already has
a `Failed` or `Stopped` variant of its own renames it.

"Accepts" means the schema, not only the name (first review): the contract of the watcher's type holds, under the
names `Failed` and `Stopped`, the schemas of the core's two types, compared as a contract hash compares schemas
(descriptions left out). A family with a `Failed { reason }` of its own is refused, not accepted and then sent
notifications it cannot decode.

Not chosen: a second handler on the `Actor` trait with a default that does nothing (two entry points, and a
notification would not be an ordinary message).

### 2. A type target is a prefix at a dot, and the empty prefix matches every type


A target with a `/` is the address of one actor. A target without one is a type prefix at a segment boundary:
`billing` matches the types `billing` and `billing.invoices`, not `billings`. So "one type" needs no kind of its own.
The empty prefix matches every type. ADR 0018 lists three kinds of target and not this one; it is the same rule
taken to its end, and a dead-letters actor needs it to see every failure, as `FailureWatcher` does today.

Not chosen: no target for every type (one watch per top-level name, and a new type is silently not covered); two
kinds of type target, exact and prefix, written differently.

**A type target tells of failures only** (second review, main session). A watch on one actor tells its watcher of
that actor's failures and of its going away. A watch on a type prefix, or on every type, tells of failures only:
its watcher gets no `Stopped`. Both reviews named the cost of the other way: the watcher of every type, which dead
letters is, would get a message, run a turn and keep an outcome for every actor of the system that goes away, and
on PostgreSQL every removal would lock that one row. Dead letters want failures, as `FailureWatcher` reports them
today; a supervisor watches the actors it started, by address. This narrows what the owner's choice of type targets
delivers, not the choice itself. A `Stopped` for type targets can be added later without a change of a schema; the
other way round, taking it away, would break watchers.

Not chosen: `Stopped` for every target (the cost above); a filter that a watch sets (the owner chose no filter; this
is a rule of the target, with nothing to set).

### 3. `Stopped` says why, and who

`Stopped` holds the actor and a cause: the actor's own turn stopped it, it was deleted, or it was removed with its
parent. A supervisor tells an actor that finished from one that was deleted.

It also holds `by` (first review, main session): the actor whose turn removed it, or nothing when the removal came
from outside a turn. For an actor that stopped, that is the actor itself; for one removed with its parent, the actor
whose turn removed the tree. `control-effects` lets a turn delete another actor; then `by` is the deleting actor,
and a supervisor tells its own restart from an operator's delete. The field is decided now because the two schemas
are part of the contract of every watcher: a field that is added later changes all their hashes.

Not chosen: the target only; a cause of its own for a delete by a turn (it says that an actor did it, not which).

### 4. The HTTP routes are one collection, `/watches`


A watch is the pair of its target and its watcher, and both can hold a `/`, so neither is a path segment.

- `GET /watches?watcher=…` or `?target=…` lists watches, paged.
- `PUT /watches` with `{target, watcher}` sets one; it is idempotent.
- `DELETE /watches?target=…&watcher=…` removes one.

The first is a read route; the other two are write routes.

Not chosen: routes below the watcher, with the target encoded into a path segment.

### A watcher removed with its target gets nothing

Follows from ADR 0018 ("a watcher that does not exist is skipped"); there is no real choice.

When a watcher is removed in the same transaction as its target, as a parent that watches its child and is itself
deleted, it gets no notification. Its watches go with it.

### 5. A watcher declares both `Failed` and `Stopped`

A watch delivers both kinds, so a watcher's type must accept both message types. Setting a watch is refused when it
lacks either. A watcher that cares about one kind writes one empty arm for the other.

Not chosen: a watcher gets only the kinds its type declares (a filter by the back door, and the store would look up
each watcher's contract at every event).

### 6. A watch from outside a turn needs a watcher that exists

In a turn the watcher is the actor itself. From outside, `ActorSystem::watch` and `PUT /watches` are refused when the
watcher's type is not registered (`404 no-such-type`, as a send answers after `tidy-the-core`), when the watcher does
not exist (`404 no-such-instance`), or when its type does not accept the two messages (`422`). The target is not
checked: ADR 0018 says a watch survives delete and re-create of its target, so a watch on an actor or a type
that does not exist yet is allowed.

Not chosen: storing a watch for a missing watcher and removing it at the first event (a mistyped address would be
accepted in silence).

### 7. What a notification holds, and how it travels

Decided by the main session; the owner did not object. The two schemas were written out after the first review:
they are frozen once a watcher is released, because they are part of its contract.

```rust
#[serde(rename_all = "camelCase")]
pub struct Failed {
    pub actor: ActorAddress,            // the actor the message was for
    pub message_id: MessageId,
    pub message_type: String,
    pub payload: Value,
    pub correlation_id: Option<String>,
    pub from: Option<ActorAddress>,     // the sender of the failed message
    pub error: String,
    pub attempts: u32,                  // how often the message was claimed: its `attempt`
}

#[serde(rename_all = "camelCase")]
pub struct Stopped {
    pub actor: ActorAddress,            // the actor that went away
    pub cause: StopCause,               // "stopped" | "deleted" | "parent"
    pub by: Option<ActorAddress>,       // decision 3
}
```

- **`actor` is the actor of the event**, not the target of the watch: a watcher of `billing` reads
  `billing.invoices/acme` there.
- **`Failed` holds the payload and the correlation id** (first review). Dead letters send a failed message again
  with the same target, type, payload and correlation id, as `DeadLettersActor` does today; without them it would
  have to read the outcome, which may be gone. It does not hold the whole envelope: the view of a message grows
  with the core, and every new field would change the contract of every watcher.
- **`Failed` holds the sender of the failed message** (third review, asked): `from`, as the view of a message names
  it. A watcher that reports a failure to the sender needs it, and a field that is added later changes the contract
  of every watcher.
- **`correlationId`, `from` and `by` are always written** (second review): `null` when there is none, as every
  field of the wire model is.
- **The two schemas stand on their own** (third review). A contract keeps the named parts of a schema by name, so a
  family with a type of its own that is called `StopCause` would make the two differ, and the watch would be
  refused. The cause is written into the schema of `Stopped` in place, and a snapshot test pins both JSON Schemas:
  a change of a dependency that alters them fails that test before it reaches a watcher.
- A notification is a message to the watcher. Its sender is the actor of the event. It is due at once, has no
  headers and no retry policy.
- **`causedBy`** (second review). A `Stopped` is caused by the message whose turn removed the actor; a delete from
  outside has none. A `Failed` has no `causedBy`: it names the failed message in its payload. The messages that a
  failed message caused are its retries, and a retry by filter skips a failed message that has one; a notification
  there would make every failure that a watcher was told of count as retried.
- **A `Failed` is kept as long as the failed message's failure** (second and third review), because it holds its
  payload. A retention has one rule for `done` and one for `failed`. The notification takes the `failed` rule of the
  failed message for both of its own outcomes: it normally ends `done`, and the `done` rule of the sender would say
  nothing about a failure. So a payload that its sender did not want kept after a failure is not kept as the
  outcome of the notification either. A failed message without a retention gives none, and a `Stopped` has none.
- **No retry policy** (second review, asked). A notification whose handler fails ends `failed` at once and tells
  nobody; an operator sees the outcome and can send it again. A fixed policy was not chosen: nothing here is frozen
  by leaving it out, the store and a node that dies already bring a message back, and a watcher's handling of the
  two messages is plain logic that a second run does not change.
- **The store marks it.** A message has a field `notification` that only the store sets, kept in the row and in the
  envelope of its outcome and shown in the views. A message with it that ends `failed` notifies nobody. A header
  would not do, because a sender sets headers. A retry of a failed notification from outside
  (`POST /messages/{id}/retry`) is an ordinary new message and is not marked (second review): only the store sets
  the mark, on what it writes itself, and the retry cannot loop, because a notification about its failure would be
  marked.
- **Its id comes from an id of the event** (first review). The runtime draws one random id for every commit and
  every delete, as it does for a send without an id, and hands it to the store. The store names the notifications
  of that event after it, with a number: the first is `<id>.0`. So a store still draws no id, two stores that get
  the same request write the same ids, and a test can name them. Not chosen: an id that the store draws (two stores
  would order the notifications of one event differently wherever a listing orders by id, and the randomised
  comparison could not compare them); an id made from the failed message's id (that id may be used again once its
  outcome is gone, and the second notification would get the id of the first).

### 8. When notifications are written

Decided by the main session; it follows from the proposal and ADR 0018.

Notifications are written by the store, in the transaction of the event.

- **A failure.** A commit that ends a message as `failed` looks up the watches of the failing actor and puts one
  `Failed` into the mailbox of each watcher. A retried attempt ends nothing, so it notifies nobody.
- **A removal.** A commit with a stop, and a delete, put one `Stopped` into the mailbox of each watcher of each actor
  they remove: the actor itself with the cause `stopped` or `deleted`, each descendant with the cause `parent`.
- **Which watches match.** A failure of the actor `billing.invoices/acme` matches the target
  `billing.invoices/acme`, the targets `billing.invoices` and `billing`, and the target for every type. Its removal
  matches the target `billing.invoices/acme` only (decision 2). Both are short lists, looked up by the primary key
  of the watches.
- **One notification per watcher and actor** (first review). A watcher that holds two watches that match a failure,
  such as `billing` and `billing.invoices/acme`, gets one message. A watcher of two actors of one tree gets one
  `Stopped` for each.
- **A notification wakes a node as a send does** (second review). The store announces it where it announces sends,
  and the runtime wakes its own loop after a commit that ended a message as `failed` or stopped its actor, and after
  a delete.
- **A fixed order** (first review). The notifications of one event are written in the order of the address of the
  actor they are about, and then of the watcher's address. Their numbers (decision 7) and their place in a mailbox
  follow that order, in every store.
- **A watcher that does not exist** is skipped and its watch is removed, in the same transaction. A watcher that the
  same transaction removes counts as not existing.
- **Removing an actor removes the watches it holds as a watcher.** The watches on it as a target stay.
- **A notification to a watcher is not a send of the turn.** A send to an actor that does not exist rejects the
  commit; a notification to a watcher that does not exist never does.
- **The watches of a turn come first** (first review). A commit sets and removes the watches of its turn before its
  creates and its sends, and before it writes notifications, in the order of the calls: after `watch` and `unwatch`
  of one target in one turn, the last call stands. A turn that fails sets none.
- **PostgreSQL keeps the one order of locks** (its turn; widened after the second review). A commit that ends a
  message as `failed`, a commit that stops its actor, and a delete first read the watchers of that one actor. They
  lock those rows in the same statement as the other rows they touch, in address order, and then read the watches
  again. A watch changes only under the lock of its watcher's row (decision 9). So a supervisor that sends to a
  worker while the worker stops does not deadlock with it.
  - A watcher in the second reading whose row exists and is not locked had its watch set in between: the
    transaction is run again, as for a deadlock. Writing to a mailbox without its lock could break the order of
    delivery, and leaving the watcher out would lose a notification.
  - A watcher whose row does not exist is skipped, and its watch is removed.
  - All these locks are taken before the clock is read, as in every other write.
  - The descendants of a removed actor are found under locks, level by level (`child-actors`). Their watchers are
    locked after the tree, in address order among themselves. That can deadlock with another commit, and is run
    again, as the removal of a tree already is.

### 9. Setting, removing and listing a watch

Decided by the main session; it follows from decisions 2, 4, 5 and 6.

- **In a turn:** `turn.watch(target)` and `turn.unwatch(target)`. The watcher is the actor that runs the turn. Both
  are buffered and applied by the commit with the turn's other effects. `watch` fails at the call when the actor's
  own type lacks the two messages.
- **A target in code** is a `WatchTarget`: one actor (from a ref or an address), the types under a prefix, or every
  type.
- **A target as text**, over HTTP and on the command line: an address (`billing.invoices/acme`), a type prefix
  (`billing`), or `*` for every type. `*` is what is stored; an empty value is easy to mistype. A text with a `/`
  must be a valid address, and one without must be `*` or a valid type name (first and third review). Anything
  else is refused: `400` over HTTP, also as a filter of the list, and an error at the call in a turn.
- **From outside:** `ActorSystem::watch`, `unwatch` and `watches`, the routes of decision 4, `ActorClient`, and
  `cicada get watches`, `cicada create watch` and `cicada delete watch`.
- **Setting a watch is idempotent.** Removing one that does not exist answers `404 no-such-watch` over HTTP and does
  nothing in a turn.
- **Every change of a watch locks the row of its watcher** (its turn): the commit of the watcher holds it already,
  and a call from outside takes it. That is what makes the reading of decision 8 exact.
- **A list** is paged in the order of target and watcher, and can be narrowed to one watcher or one target. A
  target narrows to the watches that are stored with exactly that target, not to every watch that would match an
  actor (first review). With both, the list holds the one watch of that pair, or nothing.
- **An actor may watch itself.** It is told of its own failed messages. When it stops, it is the watcher that goes
  away, so it gets nothing.
- **The refusal for a watcher whose type lacks the two messages** is `422 not-a-watcher` over HTTP (first review).

### 10. Schema: migration 5 in both stores

Decided by the main session.

- A table `watches (target, watcher)` with both as its primary key, and an index on `(watcher, target)` for the
  removal of a watcher and the list by watcher. No foreign keys, as elsewhere.
- A column `messages.notification`, false by default, and the same mark in the envelope that an outcome keeps.

The migration is not additive (its turn). A node of 0.8.0 that opens the migrated database refuses it. One that is
still running does not notice: a message that fails there notifies nobody, and an actor it removes keeps its
watches. So every node of the old version is stopped before the first new one starts. The docs of both schema
modules say so, and `docs/operations.md` names 0.9.0 as such a release.

### 11. A short ADR records the rules of a watch

ADR 0018 covers watches, but this design adds to it: the target for every type, the cause in `Stopped`, that a
watcher must accept both messages, and that the store marks a notification and draws its id.
`docs/adr/0023-the-rules-of-a-watch.md` records them. The main session wrote it at this change's turn and commits it
with the change when it is ready, as for `one-name-keys` (ADR 0021).

Not chosen: taking ADR 0018 as enough.

### 12. The randomised comparison of the stores sets watches

Decided by the main session (its turn), as for `child-actors`.

Notifications are written by each store on its own, at three events, and each store finds the watchers in its own
way. The comparison of `tests/fuzz` gets: setting and removing a watch as operations, in a commit and from outside;
the listing of watches; and the watches as part of the state that is compared at the end. The notifications are
compared with the mailboxes they land in. Their ids and their order are the same in both stores (decisions 7 and
8), so the comparison needs no rule of its own for them.

The simulation of nodes under faults gets a watcher too (second review): one actor watches the others, and a new
check says that every `failed` outcome of a watched actor has exactly one `Failed` for that watcher, whatever
commits were repeated or lost. That is the changed G4, tested where the other guarantees are. The simulation never
removes the watcher, and its check that every ended message was accepted counts a notification as accepted (third
review).

A watcher that does not exist while its watch is stored cannot be made through the calls of a store: watches follow
their watcher. So that rule is tested on SQLite and on PostgreSQL, with a row removed by hand, and not in the
conformance suite (third review; changed by decision 16).

The comparison does not run PostgreSQL. The conformance suite gets two cases for its locks (first review): a
watcher whose address sorts before its target's sends to the target while the target is deleted, and a watch is set
while a message of its target fails. Each runs many times; every call ends without an error, and the watcher has
the notifications of the watches that were stored before the event.

### 13. The terminal UI lists the watches

Decided by the main session (its turn). The owner asked for the parent and the children in the terminal UI when
`child-actors` was described ("Terminal UI should show parent and maybe even a hierarchy if possible"); the main
session takes from that that the UI shows what the core stores.

`:watches` is a listing like the others: one row per watch, with its target and its watcher, filtered as the user
types. Enter on a row opens the screen of the watcher. The UI sets and removes no watch; the command line does.

Not chosen: no view (a stored thing that the UI cannot show); the watches of an actor as a tab of its screen (two
more requests on every refresh of every actor screen, for a thing most actors do not have).

The first review asked whether the view belongs here, because the owner's remark was about another change. It
stays: it is one listing of the kind the UI has six of, and the owner was told of it when it was added.

### 14. A conformance case that depends on the speed of the machine is made steady

Decided by the main session. A finding from the release of 0.7.0, put here because this change adds conformance
cases with locks and timing of its own (the owner's rule: a small finding goes into the next change that owns the
area).

The release workflow of 0.7.0 failed once in `s7_release_expiry_and_lost_commits_keep_the_message_pending` on
PostgreSQL, on a commit that had passed CI minutes before. The case claims a message with the default lease of the
suite, which is 0.8 seconds of real time on PostgreSQL (four ticks of 200 ms), makes two more calls and then
expects the message to be `Running`. On a slow runner the lease had run out, and the message was `Pending`. A
second attempt of the same workflow run settled it.

The last claim of that case takes a lease that is long for every fixture, because nothing in the case waits for it
to end. The builder looks for the same pattern in the other cases: a claim with the default lease, on a fixture
that runs on real time, followed by an assertion that needs the lease to hold. Each gets the long lease.

Not chosen: a longer tick for PostgreSQL (every case that waits for a lease to end would wait longer, and the
pattern would stay).

### 15. Three small things left from `child-actors`

Decided by the main session. The validator of `child-actors` passed it and noted these; none blocked the release of
0.8.0. They go here by the owner's rule for small findings, because this change edits the same files.

- `docs/architecture.md` says of a stop, too, that a message "being handled" ends `purged`. The message of the
  turn that stops ends `done`.
- One line of `README.md`, in the part on children, is 181 columns long.
- The `children` tab of the terminal UI shows the first 1000 children and does not say when there are more. The
  mailbox fact of the same screen says `1000+`.

### 16. What the builder settled

Decided by the main session on the builder's report (2026-11-12). The design left these open; the build settled them,
and the conformance suite pins the first two on every store.

- **A notification whose id is in use is skipped.** The store writes the other notifications of the event, and the
  commit or delete succeeds; it logs a warning. It can only happen when a caller of the private trait reuses an event
  id, as the runtime draws a new one for every commit and every delete. Not chosen: failing the commit or the delete
  (a watch never fails the commit of the watched actor).
- **A watcher that does not exist loses all its watches**, not only the ones that matched the event. Not chosen:
  removing only the watch that matched (watches follow their watcher, and the others of a watcher that is gone could
  never tell anyone). The conformance suite shows it with a watcher that holds three watches; its fixtures remove the
  watcher's row by hand, past the store, which changes the last sentence of decision 12: the rule is tested in the
  conformance suite too, on every store.
- **`Turn::watch` and `Turn::unwatch` take `impl IntoWatchTarget`**, a public trait for a typed ref, an address, a
  `WatchTarget` or its text, so that a text that is no target fails at the call, as decision 9 asks.
- **`PUT /watches` answers `201` for a new watch and `200` for one that existed**, with the watch as its body, as
  `PUT /actors/{type}/{key}` does.

## Risks / Trade-offs

- **A watcher of every type is in the way of every failure of the system**: each locks its row on PostgreSQL and
  puts a message into its mailbox. → Failures are the rare case; removals do not reach it (decision 2).
- **A watch on one actor stays when that actor is gone for good.** → That is the rule of ADR 0018 (a watch survives
  delete and re-create). A watcher that is done with a target removes the watch, for example when it handles the
  `Stopped`; the docs say so.
- **One event can write many messages:** an actor with many watchers, or a tree with many actors that are each
  watched. → It is one transaction, like the removal of a tree itself; `docs/operations.md` says so.
- **A notification that fails is lost as a signal**: it notifies nobody and is not retried. → It still has a
  `failed` outcome, which an operator sees and can send again.
- **A `Failed` holds a copy of the payload**, and the watcher may keep it in its state. → That is what dead letters
  are for; the outcome of the notification follows the retention of the failed message (decision 7).
- **A watcher that does not handle its mailbox lets notifications pile up.** → As for any actor; `stats` and the
  mailbox views show it.
- **More locks per commit on PostgreSQL** when a watched actor fails or goes away. → Bounded by the existing re-run
  of deadlocks.
- **The watcher of a removed root is locked for the whole removal on PostgreSQL** (third review). A turn that runs
  on that watcher cannot renew its lease meanwhile, because a renewal takes the same lock. A removal that takes
  nearly as long as the lease lasts costs that turn its lease, and its handler runs again. → That needs a very
  large tree, which `child-actors` already tells a host to remove from the leaves up; `docs/operations.md` names
  this as one more reason.
- **The target for every type goes one step beyond the list of ADR 0018** (decision 2). → ADR 0023 records it
  (decision 11).
- **A node of 0.8.0 that keeps running writes no notifications.** → Decision 10: all old nodes are stopped first.
- **A watcher whose type no longer accepts the two messages**, after a new version of its code, cannot decode a
  notification. → The notification ends `failed` and notifies nobody; the operator sees the outcome. The start of a
  node does not check this.

## Migration Plan

Stop all nodes of the previous version, then start the new ones; the first one migrates the store. Nothing changes
for an actor that nobody watches. Code that builds a `Message` or a `MessageView` by hand names the new field
`notification`. A type that starts to watch adds two variants to its message family, so its contract changes: the
nodes that host it are upgraded as `docs/operations.md` says under "Changing a contract". A host that used `FailureWatcher` keeps it until `cicada-actors` replaces it.

The version is a breaking one (the schema, and the guarantees G2 and G4): 0.9.0 after 0.8.0.
