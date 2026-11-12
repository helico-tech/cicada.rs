# supervisor

## ADDED Requirements

### Requirement: A supervisor starts its children, each in a turn of its own

A supervisor SHALL have the type name `cicada.supervisor`. On `Start` it SHALL send itself one `StartChild`, with a
retry policy, for each child of its state that is not running. On `StartChild` it SHALL, in one commit, create that
child as its own child with the state of its entry, watch it, send it its start message when the entry has one, and
mark it as running; a child that is running SHALL be left as it is.

#### Scenario: Start

- **WHEN** a supervisor whose state holds two children, one with a start message, gets `Start`
- **THEN** both actors exist as its children with the states of their entries, the supervisor watches both, and the
  first has its start message in its mailbox

#### Scenario: A second start

- **WHEN** the supervisor gets `Start` again
- **THEN** no child gets a second start message

#### Scenario: One child exists elsewhere

- **WHEN** a supervisor gets `Start` with two children, and the first exists already under another parent
- **THEN** the second child is started; the `StartChild` of the first ends `failed` with the cause `commit`, and the
  first is not running

#### Scenario: The supervisor goes away

- **WHEN** a supervisor with children is deleted
- **THEN** its children are removed with it

#### Scenario: A message of the wrong family does not compile

- **WHEN** code builds a child entry for a ref with a start message of another family
- **THEN** the code does not compile

### Requirement: The supervisor's own messages count only from itself

A supervisor SHALL act on `StartChild`, `Restarted` and `Escalate` only when it sent them itself. From anybody else
they SHALL end `done` and change nothing. A `StartChild` for an address that is not one of its children SHALL change
nothing. A supervisor SHALL act on a `Failed` or a `Stopped` only when the store marked it as a notification; one
that a client sent SHALL end `done` and change nothing.

#### Scenario: An escalation sent by a client

- **WHEN** a client sends a supervisor `Escalate`
- **THEN** the message ends `done`, and a supervisor above restarts nothing

#### Scenario: A redrive of an escalation

- **WHEN** dead letters recorded the failed `Escalate` of a supervisor, and an operator redrives it
- **THEN** the redriven message ends `done`, and nothing is restarted

#### Scenario: A start of an actor that is not a child

- **WHEN** a supervisor sends itself `StartChild` for an address that its state does not hold
- **THEN** nothing is created

#### Scenario: A `Failed` sent by a client

- **WHEN** a client sends a supervisor a `Failed` with the cause `handler` about one of its children
- **THEN** the message ends `done`, and nothing changes: the child is not restarted

#### Scenario: A `Stopped` sent by a client

- **WHEN** a client sends a supervisor a `Stopped` about one of its children
- **THEN** the message ends `done`, and nothing changes: the supervisor still holds and watches the child

### Requirement: A start is tried again, and then the child is given up without a restart above

While attempts of a `StartChild` are left, a call that fails SHALL make the attempt fail, so that it is retried. At
the last attempt, a refused call SHALL mark the child as given up with the error and SHALL make the supervisor
escalate with an error that starts with `cannot start`, refused for good, so that nobody restarts for it. A failed
read of the store at the last attempt SHALL fail the `StartChild` with the cause `node` and change nothing.

#### Scenario: A type that is registered a little later

- **WHEN** a supervisor gets `Start` while the type of a child is not registered yet, and the type is registered ten
  seconds later
- **THEN** the child is started by a later attempt of its `StartChild`, and nothing is given up

#### Scenario: A type that never comes

- **WHEN** the type of a child is not registered by the last attempt of its `StartChild`
- **THEN** the child is marked as given up, and a message of the supervisor ends `failed` with the cause `refused`
  and an error that starts with `cannot start` and names the child

#### Scenario: A supervisor above is not disturbed

- **WHEN** a supervisor that is the child of another supervisor cannot start one of its children
- **THEN** the supervisor above does not restart it

#### Scenario: A start message the type does not take

- **WHEN** the type of a child of the state does not take its start message, at every attempt
- **THEN** the child is created and runs without it, and it is marked as given up

#### Scenario: A child that was given up gets a new try

- **WHEN** a supervisor gets `Start` again after it gave up on a child that is not running, and the child can be
  started now
- **THEN** the child is started, and it is no longer given up

#### Scenario: A stuck restart is ended by a start

- **WHEN** a supervisor restarts a child, an operator purges the supervisor's mailbox before `Restarted` is handled,
  and the supervisor then gets `Start`
- **THEN** the restart is ended, and the next failure of the child with the cause `handler` restarts it

#### Scenario: A child that was given up and still runs is taken back

- **WHEN** a supervisor gets `Start` again after it gave up on a child that still runs
- **THEN** the child keeps its state and its mailbox, it is no longer given up, its restarts are cleared, and its
  next failure with the cause `handler` restarts it

### Requirement: A supervisor restarts a child that is at fault

When a message of a child ends `failed` with the cause `handler` or `timeout`, a supervisor SHALL in one commit
delete the child, create it again with the state of its entry, send it its start message, and send itself
`Restarted` for it, in that order, and SHALL keep the id of that `Restarted`. For the cause `state` it SHALL give up
on the child and escalate without a restart above. For every other cause it SHALL do nothing. A `Failed` about an
actor that is not its child SHALL be ignored.

#### Scenario: A restart

- **WHEN** a message of a child with a changed state and two pending messages ends `failed` with the cause `handler`
- **THEN** the child has the state of its entry again, its two pending messages ended `purged`, it has its start
  message in its mailbox, and the supervisor still watches it

#### Scenario: A turn that ran too long

- **WHEN** a message of a child ends `failed` with the cause `timeout`
- **THEN** the child is restarted

#### Scenario: A state that cannot be read

- **WHEN** a message of a child ends `failed` with the cause `state`
- **THEN** the child keeps its state and its mailbox, the supervisor marks it as given up, and a message of the
  supervisor ends `failed` with the cause `refused` and an error that starts with `cannot read`

#### Scenario: A message the child refused

- **WHEN** a message of a child ends `failed` with the cause `refused`
- **THEN** the child keeps its state and its pending messages

#### Scenario: A message that cannot be decoded

- **WHEN** a client sends a child a message it cannot decode
- **THEN** the child keeps its state and its pending messages

#### Scenario: A commit that is rejected

- **WHEN** a message of a child ends `failed` with the cause `commit`
- **THEN** the child is not restarted

#### Scenario: A failure of the store

- **WHEN** a message of a child ends `failed` with the cause `node`
- **THEN** the child is not restarted

#### Scenario: The claim limit

- **WHEN** a message of a child ends `failed` with the cause `claim-limit`
- **THEN** the child keeps its state and its pending messages, and no restart is counted

#### Scenario: An actor that is not a child

- **WHEN** a supervisor that also watches another actor gets a `Failed` about it
- **THEN** it changes nothing

#### Scenario: A state that can no longer be created

- **WHEN** the create of a restart is refused because the state of the entry does not decode
- **THEN** the child is deleted and not created again, the supervisor marks it as given up, and it escalates
  without a restart above

#### Scenario: The store fails during a restart

- **WHEN** a read of the store fails while the supervisor handles the `Failed` of a child
- **THEN** that `Failed` ends `failed` with the cause `node`, nothing is committed, and the child keeps running

### Requirement: One incident is one restart

From a restart of a child until it handles the `Restarted` whose id it kept, a supervisor SHALL ignore every `Failed`
about that child. It SHALL ignore a `Stopped` with the cause `deleted` whose `by` is the supervisor itself.

#### Scenario: A child with a backlog

- **WHEN** five queued messages of a child fail one after the other before the supervisor handles the first `Failed`
- **THEN** the child is restarted once, and one restart is counted

#### Scenario: The old life of a restarted child

- **WHEN** a supervisor restarts a child and then gets the `Stopped` of the life it deleted
- **THEN** it still holds the child and still watches it

#### Scenario: A clock that stands still

- **WHEN** a child with a backlog of failures is restarted while the time of the store does not move
- **THEN** it is restarted once, and the supervisor still holds it

#### Scenario: The new life fails

- **WHEN** a restarted child fails a message after the supervisor handled `Restarted`
- **THEN** the child is restarted again, and two restarts are counted

#### Scenario: A `Restarted` of another restart

- **WHEN** a supervisor handles a `Restarted` from itself whose id is not the one it kept for the child
- **THEN** the restart under way is not ended, and failures of the child are still ignored

### Requirement: A child that goes away is forgotten

When a child stops itself, or is deleted by anybody but the supervisor, the supervisor SHALL remove its entry and
its watch. When that happens while a restart of the child is under way, the supervisor SHALL also delete the life
its restart created.

#### Scenario: A child stops itself

- **WHEN** a child stops in a turn of its own
- **THEN** the supervisor's state no longer holds it, and the supervisor no longer watches its address

#### Scenario: An operator deletes a child

- **WHEN** an operator deletes a child
- **THEN** the supervisor forgets it and does not create it again

#### Scenario: An operator deletes a child while one of its failures waits

- **WHEN** a message of a child fails, an operator deletes the child, and the supervisor then handles the `Failed`
  and the `Stopped`
- **THEN** the child does not exist, and the supervisor's state no longer holds it

#### Scenario: A child that stopped itself just before its restart

- **WHEN** a child fails a message and then stops itself, and the supervisor handles the `Failed` and then the
  `Stopped`
- **THEN** the child does not exist, and the supervisor's state no longer holds it

### Requirement: A supervisor gives up after too many restarts, and says so upward

A supervisor SHALL restart one child at most `maxRestarts` times within `withinSeconds`, 3 times in 60 seconds unless
its state says otherwise. When the child fails once more, the supervisor SHALL NOT restart it, SHALL mark it as given
up, and SHALL send itself `Escalate`, whose handling SHALL fail with the cause `handler` and an error that starts
with `restart limit:` and names the child. A later failure of a child that is given up SHALL do nothing.

#### Scenario: The fourth failure in a minute

- **WHEN** a child with the default limit fails four times within 60 seconds, each time after it was restarted
- **THEN** it was restarted three times, and after the fourth failure a message of the supervisor ends `failed`
  with the cause `handler` and an error that starts with `restart limit:` and names the child

#### Scenario: A supervisor above is told

- **WHEN** a supervisor that is itself the child of another supervisor gives up on a child after too many restarts
- **THEN** the supervisor above restarts it, and its children exist again with the states of their entries

#### Scenario: Failures that are far apart

- **WHEN** a child fails four times, each 30 seconds after the one before
- **THEN** it is restarted four times, and the supervisor does not give up

#### Scenario: A child that is given up fails again

- **WHEN** a child that the supervisor gave up on fails once more
- **THEN** the supervisor does nothing, and the child keeps running
