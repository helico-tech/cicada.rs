# watches Specification

## Purpose
An actor can watch another actor, the actors of a type, or every actor, and is told with a message when a message
to a watched actor fails and when a watched actor goes away.

## Requirements

### Requirement: A watch is a stored pair of a target and a watcher

A watch SHALL be the pair of a target and a watcher, where the watcher is one actor. A turn SHALL set and remove a
watch of its own actor as an effect of its commit. `ActorSystem` and the HTTP API SHALL set, remove and list watches
from outside a turn. Setting a watch that exists SHALL change nothing.

#### Scenario: A turn sets a watch

- **WHEN** a turn of the actor `supervisor/a` watches the actor `worker/w1` and commits
- **THEN** the list of watches holds the pair of the target `worker/w1` and the watcher `supervisor/a`

#### Scenario: A turn removes a watch

- **WHEN** a turn of an actor that watches `worker/w1` calls `unwatch` for it and commits, and a message to
  `worker/w1` fails afterwards
- **THEN** the watch is not stored any more, and the actor is not told

#### Scenario: A turn removes a watch it does not hold

- **WHEN** a turn calls `unwatch` for a target its actor does not watch
- **THEN** the commit succeeds and nothing changes

#### Scenario: A turn sets and removes one watch

- **WHEN** a turn calls `watch` and then `unwatch` for the same target, and commits
- **THEN** no watch is stored: the last call stands

#### Scenario: A turn that fails sets no watch

- **WHEN** a turn watches an actor and then fails
- **THEN** no watch is stored

#### Scenario: Setting and removing over HTTP

- **WHEN** a client puts the pair of a target and an existing watcher to `/watches`, twice, and then deletes it
- **THEN** both puts succeed, the list holds the watch once, and after the delete the list does not hold it

#### Scenario: Removing a watch that does not exist

- **WHEN** a client deletes a watch that is not stored
- **THEN** the answer is `404 no-such-watch`

### Requirement: A target is one actor, the types under a prefix, or every type

A target with a `/` SHALL be the address of one actor. A target without one SHALL be a type prefix at a segment
boundary: it matches the type of that name and every type whose name continues with a dot. The target `*` SHALL
match every type. A target SHALL NOT need to exist. A text that is none of the three SHALL be refused.

#### Scenario: A prefix at a dot

- **WHEN** an actor watches the target `billing`, and messages fail for actors of the types `billing`,
  `billing.invoices` and `billings`
- **THEN** it is told of the first two failures and not of the third

#### Scenario: Every type

- **WHEN** an actor watches the target `*`, and a message fails for an actor of any type
- **THEN** it is told of that failure

#### Scenario: A text that is no target

- **WHEN** a client puts a watch with the target `billing..x/`, or a turn calls `watch` with it
- **THEN** the answer over HTTP is `400`, the call in the turn fails, and no watch is stored

#### Scenario: A target that does not exist yet

- **WHEN** a watch is set on the actor `worker/w9`, which does not exist, and `worker/w9` is created later and a
  message to it fails
- **THEN** the watcher is told of that failure

### Requirement: An actor may watch itself

An actor SHALL be able to watch its own address. It SHALL be told of its own messages that fail. When it goes away
it SHALL get nothing, because its watches go with it.

#### Scenario: An actor watches itself

- **WHEN** an actor watches its own address, one of its messages fails, and later it stops
- **THEN** it has one `Failed` message for that failure, and no `Stopped` is written when it stops

### Requirement: A watch on a type tells of failures only

A watch on one actor SHALL tell its watcher of that actor's failures and of its going away. A watch on a type
prefix, or on every type, SHALL tell its watcher of failures only: it SHALL get no `Stopped`.

#### Scenario: An actor of a watched type goes away

- **WHEN** one actor watches the target `billing`, another watches `billing.invoices/acme`, and
  `billing.invoices/acme` is deleted
- **THEN** the second has one `Stopped` message, and the first has none

### Requirement: A watcher is told when a message to a watched actor fails

When a message to a watched actor ends as `failed`, the same transaction SHALL put one `Failed` message into the
mailbox of each watcher. It SHALL hold the actor, the id, the type name, the payload, the correlation id and the
sender of the failed message, the error, the number of attempts and the cause. A message that ends otherwise, and
an attempt that is retried, SHALL notify nobody.

#### Scenario: A message fails for good

- **WHEN** a message to a watched actor fails on its last attempt
- **THEN** each watcher has one `Failed` message in its mailbox, with the actor, the id, the type name, the payload,
  the correlation id and the sender of the failed message, the error, the number of attempts and the cause; its
  sender is the actor, and it has no `causedBy`

#### Scenario: A failed message whose failure is not kept

- **WHEN** a message whose retention removes a `failed` outcome fails for a watched actor, and the watcher handles
  the `Failed`
- **THEN** the outcome of the `Failed` message is removed as well, although it ended as `done`

#### Scenario: A failed message without a correlation id

- **WHEN** a message without a correlation id fails for a watched actor
- **THEN** the `Failed` message has `correlationId` with the value null, and `from` with the value null when the
  failed message was sent from outside a turn

#### Scenario: A retry by filter after a watcher was told

- **WHEN** a message to a watched actor failed, its watcher was told, and an operator retries the failed messages of
  that actor by filter
- **THEN** the failed message is sent again: the notification does not make it count as retried

#### Scenario: An attempt that is retried

- **WHEN** a message to a watched actor fails and is retried under its retry policy
- **THEN** no watcher is told until the last attempt fails

#### Scenario: A message that succeeds

- **WHEN** a message to a watched actor ends as `done`
- **THEN** no watcher is told

### Requirement: A watcher is told when a watched actor goes away

When an actor that is watched by its address is stopped by its own turn, deleted from outside a turn or by the turn
of another actor, or removed with its parent, the same transaction SHALL put one `Stopped` message into the mailbox
of each watcher. It SHALL hold the actor, the cause (`stopped`, `deleted` or `parent`) and the actor whose turn
removed it, which is absent for a removal from outside a turn.

#### Scenario: An actor stops itself

- **WHEN** a turn of a watched actor stops it
- **THEN** each watcher has one `Stopped` message with the cause `stopped`, `by` is the actor itself, and its
  `causedBy` is the message whose turn stopped it

#### Scenario: An actor is deleted

- **WHEN** a watched actor is deleted over HTTP
- **THEN** each watcher has one `Stopped` message with the cause `deleted`, and `by` has the value null

#### Scenario: An actor is deleted by a turn

- **WHEN** a turn of another actor deletes a watched actor
- **THEN** each watcher has one `Stopped` message with the cause `deleted`, `by` is the actor whose turn deleted it,
  and its `causedBy` is the message of that turn

#### Scenario: A delete wakes the watcher

- **WHEN** a watched actor is deleted on a node that hosts its watcher, on virtual time
- **THEN** the watcher handles the `Stopped` at once, not after the next poll

#### Scenario: A child is removed with its parent

- **WHEN** a parent is deleted and its child is watched by a third actor
- **THEN** that actor has one `Stopped` message for the child, with the cause `parent`

#### Scenario: A child is removed with a parent that stops

- **WHEN** a turn of a parent stops it, and its child is watched by a third actor
- **THEN** that actor has one `Stopped` message for the child, with the cause `parent`, and `by` is the parent

#### Scenario: A watcher that is removed with its target

- **WHEN** a parent watches its child and the parent is deleted
- **THEN** no notification is written for the parent, and its watch is gone

### Requirement: One event writes one notification per watcher and actor, with fixed ids and a fixed order

A watcher that holds several watches that match one failure SHALL get one notification for it, and a watcher of
several actors of one removal SHALL get one for each. The notifications of one event SHALL be written in the order
of the address of the actor they are about and then of the watcher's address. Each SHALL be named after the id of the event, which the caller hands to the store, followed by its number
in that order, so that every store writes the same ids.

#### Scenario: Two watches that match

- **WHEN** an actor watches both `billing` and `billing.invoices/acme`, and a message to `billing.invoices/acme` fails
- **THEN** it has one `Failed` message, not two

#### Scenario: The ids of one event

- **WHEN** a store is given a delete with the event id `e1`, of a parent with one child, where `w/a` watches the
  child and `w/b` watches both
- **THEN** it writes three notifications named `e1.0`, `e1.1` and `e1.2`, in the order of the removed actor's
  address and then the watcher's, and the in-memory store and SQLite write the same

### Requirement: A notification that fails notifies nobody

A message that the store wrote as a notification SHALL be marked as one, and only the store SHALL set that mark.
When such a message ends as `failed`, no watcher SHALL be told.

#### Scenario: Two actors that watch each other

- **WHEN** two actors watch each other, and a message to the first fails, and the second fails to handle the
  notification
- **THEN** the first actor gets no message about that, and nothing more is written

#### Scenario: A failed notification is sent again from outside

- **WHEN** a notification ended as `failed`, an operator retries it with `POST /messages/{id}/retry`, and the new
  message fails too
- **THEN** the new message is not marked as a notification, the watchers of that watcher are told of its failure
  with a marked message, and nothing more follows from that one

#### Scenario: A sender cannot mark a message

- **WHEN** a client sends a message of the type `Failed` to a watched actor over HTTP, and it fails
- **THEN** the watchers of that actor are told, because the message is not marked as a notification

### Requirement: A watch never fails the commit of the watched actor

A watcher that does not exist SHALL be skipped, and its watch SHALL be removed in the same transaction. The commit
of the watched actor SHALL succeed all the same.

#### Scenario: A watcher that is gone

- **WHEN** a watch names a watcher whose row was removed by hand in a SQLite or PostgreSQL database, and a message
  to the target fails
- **THEN** the commit succeeds, the message ends as `failed`, and the watch is removed

### Requirement: Watches follow the life of their watcher, not of their target

Removing an actor SHALL remove every watch it holds as a watcher. A watch on an actor SHALL stay when that actor is
deleted and created again.

#### Scenario: The watcher is deleted

- **WHEN** an actor that watches three targets is deleted
- **THEN** none of the three watches is stored any more

#### Scenario: The target is deleted and created again

- **WHEN** a watched actor is deleted, created again, and a message to it fails
- **THEN** the watcher is told of the `Stopped` and of the `Failed`

### Requirement: A watcher accepts both notifications

A watch SHALL be refused when the contract of its watcher's type does not hold both message types, `Failed` and
`Stopped`, with the schemas of the core's two types. A watch from outside a turn SHALL also be refused when the
watcher does not exist.

#### Scenario: A turn of an actor without the two messages

- **WHEN** a turn of an actor whose message family has no `Stopped` calls `watch`
- **THEN** the call fails, and the turn can go on

#### Scenario: A watcher that does not exist

- **WHEN** a client puts a watch whose watcher does not exist
- **THEN** the answer is `404 no-such-instance`

#### Scenario: A watcher whose type is not registered

- **WHEN** a client puts a watch whose watcher is of a type that is not registered
- **THEN** the answer is `404 no-such-type`

#### Scenario: A watcher whose type lacks a message

- **WHEN** a client puts a watch whose watcher exists and whose type has no `Failed`
- **THEN** the answer is `422 not-a-watcher`, and no watch is stored

#### Scenario: A watcher with a `Failed` of its own

- **WHEN** a client puts a watch whose watcher's type has a message type `Failed` with another schema than the
  core's
- **THEN** the answer is `422 not-a-watcher`, and no watch is stored

### Requirement: Watches are listed

`GET /watches` SHALL list watches in the order of target and watcher, paged, and SHALL narrow the list to one watcher
or one target when asked. It SHALL be a read route. The command line SHALL list, set and remove watches.

#### Scenario: The watches of one watcher

- **WHEN** a client gets `/watches?watcher=supervisor/a`
- **THEN** the answer holds the watches of that watcher and no other

#### Scenario: The watches of one target

- **WHEN** the watches with the targets `billing` and `billing.invoices/acme` are stored, and a client gets
  `/watches?target=billing`
- **THEN** the answer holds the first watch only: a target narrows to the watches stored with exactly that target

#### Scenario: Both filters, and a bad one

- **WHEN** a client gets `/watches?target=billing&watcher=dead-letters/_`, and then `/watches?target=a//b`
- **THEN** the first answer holds that one watch when it is stored and nothing otherwise, and the second is `400`

#### Scenario: The command line

- **WHEN** `cicada create watch --target billing --watcher dead-letters/_` is run, and then `cicada get watches`
- **THEN** the table holds the target `billing` and the watcher `dead-letters/_`

### Requirement: The terminal UI lists the watches

`cicada tui` SHALL show the watches as a listing of its own, one row per watch with its target and its watcher,
filtered as the user types. Enter on a row SHALL open the screen of the watcher.

#### Scenario: The view of watches

- **WHEN** the user types `:watches` and Enter, and then presses Enter on a row
- **THEN** the UI shows the stored watches, and then the screen of the watcher of that row

### Requirement: The stores migrate to the table of watches

Both SQL stores SHALL add the table of watches and the mark on a message in an appended migration. Messages and
outcomes that existed before the migration SHALL not be marked as notifications.

#### Scenario: A database of schema version 4 is opened

- **WHEN** a store opens a database of schema version 4 that holds a pending message and an outcome
- **THEN** the database is at the schema version this store writes, the message and the outcome are still there,
  neither is marked as a notification, and no watch is stored

### Requirement: A `Failed` says why the message failed

Every `Failed` SHALL hold exactly one cause, which says why the message failed: `handler`, `node`, `refused`,
`timeout`, `message`, `state`, `commit` or `claim-limit`. The scenarios below say which cause each way of failing
has.

#### Scenario: A handler fails

- **WHEN** the handler of a message to a watched actor returns an error and the message has no retry left
- **THEN** the `Failed` of each watcher has the cause `handler`

#### Scenario: A handler refuses a message

- **WHEN** the handler fails for good
- **THEN** the cause is `refused`

#### Scenario: A turn runs too long

- **WHEN** the turn is cut off at its maximum duration and the message has no retry left
- **THEN** the cause is `timeout`

#### Scenario: A message that cannot be decoded

- **WHEN** a message with a payload that does not fit ends `failed`
- **THEN** the cause is `message`

#### Scenario: A state that cannot be decoded

- **WHEN** a message to an actor whose state cannot be decoded ends `failed`
- **THEN** the cause is `state`

#### Scenario: A commit that is rejected

- **WHEN** a handler returns after a send to an actor that does not exist
- **THEN** the cause is `commit`

#### Scenario: The claim limit

- **WHEN** the claim limit fails a message
- **THEN** the cause is `claim-limit`

#### Scenario: An address the type cannot read

- **WHEN** a message to an actor whose address its type cannot read ends `failed`
- **THEN** the cause is `state`

#### Scenario: An effect that is refused after the handler returned

- **WHEN** a handler returns and the node then refuses one of its effects
- **THEN** the cause is `commit`

#### Scenario: A read of the store that fails after the handler returned

- **WHEN** a handler returns, and the read of the registry that the node then needs fails until the node gives up
- **THEN** the message ends `failed` with the cause `node`

#### Scenario: A handler panics

- **WHEN** the handler of a message without a retry policy panics
- **THEN** the cause is `handler`

#### Scenario: An observer is not touched

- **WHEN** an observer is told the end of a turn that failed
- **THEN** it gets the outcome `failed` with the error, as before

#### Scenario: A handler passes a failed read of the store on

- **WHEN** a handler without a retry policy returns the `TurnError::Store` of a `send_raw` with `?`, or wraps it
  with a context of its own
- **THEN** the cause is `node`

#### Scenario: A handler refuses with a store error inside

- **WHEN** a handler refuses a message for good with an error that holds a store error
- **THEN** the cause is `refused`

#### Scenario: A turn that ran too long after a refusal

- **WHEN** a handler refuses a message for good, but only after its turn ran past its maximum duration
- **THEN** the cause is `timeout`

#### Scenario: A retry the store cannot schedule

- **WHEN** a handler fails, its retry policy asks for a retry, and the store refuses the time of the retry
- **THEN** the message ends `failed` with the cause `handler`

#### Scenario: A new state that cannot be stored

- **WHEN** a handler returns with a new state that cannot be encoded
- **THEN** the cause is `handler`

#### Scenario: A panic outside the handler

- **WHEN** the node panics around a turn, outside the handler
- **THEN** the message ends `failed` with the cause `node`
