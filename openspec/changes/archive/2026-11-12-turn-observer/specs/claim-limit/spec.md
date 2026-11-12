# claim-limit

## ADDED Requirements

### Requirement: A claim counts the claim it replaces as lost

A message SHALL carry `lostClaims`, 0 when it is accepted. A claim of an actor that still holds a lease that was
neither committed nor given back SHALL add 1 to the `lostClaims` of the message of that lease, when that message is
still pending, in the transaction of the claim. Nothing else SHALL change `lostClaims`. The view of a message over
HTTP SHALL show it, and so SHALL the envelope of an outcome.

#### Scenario: A lease runs out and the message is claimed again

- **WHEN** a message is claimed, its lease runs out, and the actor is claimed again
- **THEN** the second claim returns the message with an `attempt` of 2 and a `lostClaims` of 1

#### Scenario: A lease that ran out and is not replaced

- **WHEN** a message is claimed and its lease runs out, and no other claim comes
- **THEN** the message has a `lostClaims` of 0

#### Scenario: A release is not counted

- **WHEN** a message is claimed, the claim is released, and the message is claimed again
- **THEN** the second claim returns it with an `attempt` of 2 and a `lostClaims` of 0

#### Scenario: A committed retry is not counted

- **WHEN** a message is claimed, the claim is committed as a retry, and the message is claimed again
- **THEN** the second claim returns it with an `attempt` of 2, 1 failure and a `lostClaims` of 0

#### Scenario: A commit after the lease ran out

- **WHEN** a message is claimed, its lease runs out, and the turn then commits `done` before another claim comes
- **THEN** the commit succeeds and the message in the outcome has a `lostClaims` of 0

#### Scenario: The message of the lost claim is gone

- **WHEN** a message is claimed, its lease runs out, the message is cancelled, and the next message of the actor is
  claimed
- **THEN** the claimed message has a `lostClaims` of 0

#### Scenario: The view of a message

- **WHEN** a client reads a message with two lost claims
- **THEN** the view has `lostClaims: 2`, and the wide table of `cicada get messages` shows it in the column `LOST`

#### Scenario: A view from an earlier node

- **WHEN** a client reads a view of a message that has no `lostClaims`
- **THEN** it reads a `lostClaims` of 0

#### Scenario: The outcome keeps the count

- **WHEN** a message with one lost claim ends `done`
- **THEN** the message in its outcome has a `lostClaims` of 1

### Requirement: The stores migrate to the count of lost claims

Both SQL stores SHALL add the count in an appended migration that gives every message that exists a `lostClaims` of 0.

#### Scenario: A database of schema version 5 is opened

- **WHEN** a store opens a database of schema version 5 that holds a pending message with an `attempt` of 7
- **THEN** the database is at the schema version this store writes, and the message is still there with an `attempt`
  of 7 and a `lostClaims` of 0

### Requirement: A message with too many lost claims ends `failed`

A node whose limit is `n` and that claims a message with `n` or more lost claims SHALL NOT decode or run it. It
SHALL commit the message as `failed` with the error `claim limit: <n> lost claims`, where `<n>` is its `lostClaims`,
without asking the retry policy of the message. It SHALL tell its observers of that claim only after the commit
succeeded: the start and then the end.

#### Scenario: A message whose turns keep dying

- **WHEN** a node with a limit of 3 claims a message for the fourth time, after three leases of it ran out
- **THEN** the message ends `failed` with the error `claim limit: 3 lost claims`, and its handler does not run a
  fourth time

#### Scenario: Fewer lost claims

- **WHEN** a node with a limit of 3 claims a message with two lost claims
- **THEN** the handler runs

#### Scenario: Claims that were given back do not count

- **WHEN** a node with a limit of 3 claims a message whose five earlier claims were all released
- **THEN** the handler runs

#### Scenario: Runs that were committed do not count

- **WHEN** a node with a limit of 3 claims a message whose handler failed five times, each committed as a retry
- **THEN** the handler runs

#### Scenario: The retry policy is not asked

- **WHEN** a message with a retry policy that has attempts left reaches the limit
- **THEN** it ends `failed` and is not rescheduled

#### Scenario: The mailbox moves on

- **WHEN** the head message of an actor ends `failed` by the limit and another message is pending
- **THEN** the next message is handled

#### Scenario: A watcher is told

- **WHEN** a message to a watched actor ends `failed` by the limit
- **THEN** each watcher gets one `Failed` with the error of the limit and, as `attempts`, the `attempt` of the claim
  that failed it

#### Scenario: An observer is told after the commit

- **WHEN** a node with an observer fails a message by the limit
- **THEN** the observer is told the start and then the end, with the outcome `failed`, and both after the message
  has its outcome

#### Scenario: The commit of the limit does not succeed

- **WHEN** a node with a limit of 3 claims a message with three lost claims, and its commit cannot reach the store
  until the lease has run out
- **THEN** the observers are told nothing of that claim, and the next claim ends the message `failed` with the
  error `claim limit: 4 lost claims`

#### Scenario: The message is sent again

- **WHEN** an operator retries a message that the limit failed
- **THEN** the new message has a `lostClaims` of 0 and its handler runs

### Requirement: A node gives back a claim that arrives too late

A node that receives a claim after the local deadline of its lease (the time the claim was sent, plus the lease TTL,
less the safety margin) SHALL NOT call an observer, load or run anything for it, and SHALL release it, so that the
claim is not lost. A node that receives a claim with less than one heartbeat of its lease left SHALL renew the lease
at once, not one heartbeat later.

#### Scenario: A claim that arrives too late

- **WHEN** the answer of a claim arrives after its lease deadline
- **THEN** the node releases the claim, its handler does not run, and the next claim of the message has the same
  `lostClaims` as before

#### Scenario: A claim that arrives shortly before its deadline

- **WHEN** the answer of a claim arrives with less than a heartbeat of its lease left
- **THEN** the node renews the lease at once, the handler runs to the end and commits, and no claim is lost

### Requirement: The limit of lost claims is set on the node

`NodeConfig::max_lost_claims` SHALL be the limit of a node: 5 unless it is set, and none when it is `None`. A node
with a limit of 0 SHALL NOT start.

#### Scenario: The default

- **WHEN** a node with the default configuration claims a message with five lost claims
- **THEN** the message ends `failed` with the error `claim limit: 5 lost claims`

#### Scenario: No limit

- **WHEN** a node whose `max_lost_claims` is `None` claims a message with twenty lost claims
- **THEN** the handler runs

#### Scenario: A limit of zero

- **WHEN** a node is started with a `max_lost_claims` of 0
- **THEN** the start fails with a problem that names `max_lost_claims`
