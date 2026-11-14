# value-depth Specification

## Purpose
How deep a payload or a state may nest (64 levels of arrays and objects, ADR 0029), where a deeper one is refused,
and what that promises: no value a node accepts ends it by its depth, and every view of one can be read back.

## Requirements

### Requirement: A payload and a state are at most 64 levels deep

A payload or a state that holds more than 64 arrays and objects inside each other SHALL be refused: from outside a
turn with `OpError::Invalid`, over HTTP with `400`, and in a turn at the call with `TurnError::Invalid`. A turn whose
new state is too deep SHALL end its message `failed` with the cause `handler`, and an error that starts with `State
cannot be stored:`. The error SHALL name the payload or the state and the limit.

#### Scenario: A send over HTTP

- **WHEN** a client sends a payload 65 levels deep, to a node that hosts the type and to one that does not
- **THEN** both answer `400`, and the detail says the payload is nested deeper than 64 levels

#### Scenario: A create over HTTP

- **WHEN** a client creates an actor with a state 65 levels deep
- **THEN** the answer is `400`, and nothing is created

#### Scenario: A value at the limit

- **WHEN** a client sends a payload 64 levels deep that the type takes
- **THEN** it is accepted and handled

#### Scenario: A send and a create in a turn

- **WHEN** a handler sends a payload 65 levels deep, or creates an actor with a state 65 levels deep, by a typed ref
  or as raw JSON
- **THEN** each call returns `TurnError::Invalid`

#### Scenario: A new state that is too deep

- **WHEN** a handler returns with a new state 65 levels deep
- **THEN** the message ends `failed` with the cause `handler` and an error that starts with `State cannot be stored:`,
  and the stored state is as it was

### Requirement: The check never goes deeper than the limit

The check of a payload or a state SHALL stop at the first level beyond the limit, so that it refuses a value of any
depth with an error.

#### Scenario: A very deep value

- **WHEN** the check is given a value 100,000 levels deep, on a thread with a stack of 256 KiB
- **THEN** it refuses the value, naming the limit (the test leaks the value, whose drop would overflow)

#### Scenario: A deep value sent from code

- **WHEN** code sends a payload 1,000 levels deep through `ActorSystem::send_message`, and creates an actor with a
  state that deep through `ActorSystem::create_actor`
- **THEN** both are refused with `OpError::Invalid`, and the node goes on serving

### Requirement: An operator's retry checks the payload

A retry of a message SHALL refuse a stored payload deeper than the limit with `RetryResult::TooDeep`, over HTTP
`409 message-not-retryable` with a detail that names the limit, and write nothing. A bulk retry SHALL skip such a
message, not count it, and go on with the others.

#### Scenario: A retry of a deep message from before

- **WHEN** a message with a payload 100 levels deep, written before this release, ended `failed`, and an operator
  retries it
- **THEN** the answer is `409 message-not-retryable`, and no message is written

#### Scenario: A bulk retry

- **WHEN** an operator retries that message and two failed messages within the limit in one bulk retry
- **THEN** the two are retried, the answer counts two, and the deep one is not retried

#### Scenario: A retried notification at the limit

- **WHEN** a `Failed` notification 65 levels deep ended `failed`, and an operator retries it
- **THEN** the answer is `409 message-not-retryable`

### Requirement: Every value accepted from this release on can be read back over HTTP

Every view of a message, a notification, an outcome or an actor SHALL be readable by `cicada-client`, for every value
written from this release on.

#### Scenario: A message at the limit

- **WHEN** a payload 64 levels deep is pending and a client reads `GET /messages/{id}` with `cicada-client`, and
  after it is handled reads the page of `GET /outcomes` that holds it
- **THEN** the client reads both views and the payload

#### Scenario: The deepest view

- **WHEN** a message with a payload 64 levels deep ends `failed`, its actor is watched by an actor whose handler fails
  the `Failed`, and a client reads the notification with `GET /messages/{id}` and the page of `GET /outcomes` that
  holds its outcome
- **THEN** the client reads both views and the payload of the failed message in them

#### Scenario: An actor at the limit

- **WHEN** an actor has a state 64 levels deep and a client reads `GET /actors/{type}/{key}`
- **THEN** the client reads the view and the state

### Requirement: A value at the limit decodes on a thread of the runtime

A value 64 levels deep of a list that holds itself, of a tagged enum that holds itself, and of a type with six
newtypes between two of its levels SHALL be decoded on a thread of the runtime without overflowing its stack, in a
debug build.

#### Scenario: Three shapes at the limit

- **WHEN** values 64 levels deep of those three shapes are sent to a node that hosts their types, in a debug build on
  threads with their default stack
- **THEN** each is decoded, handled and committed, and the process goes on

### Requirement: Values stored before keep working

A pending message or a stored state that is deeper than the limit SHALL be delivered and decoded as before. Only a
value that is written from now on SHALL be checked.

#### Scenario: A deep message from before

- **WHEN** a store holds a pending message 100 levels deep, written before this release
- **THEN** a node of this release delivers it, and the handler gets it

#### Scenario: A deep state from before, left as it is

- **WHEN** an actor has a stored state 100 levels deep, written before this release, and its handler handles a
  message without changing the state
- **THEN** the message ends `done`

#### Scenario: A deep state from before, written back

- **WHEN** the handler of that actor writes the state back as it is
- **THEN** the message ends `failed` with the cause `handler`, and the stored state is as it was

### Requirement: A typed watcher reads a failure notice for a payload at the limit

A failed message with an accepted payload of depth 64 SHALL produce a readable Failed notice of depth 65. Every
backend SHALL retain and claim the notice's original payload, and a typed watcher SHALL decode it both locally and
through an HTTP remote worker, on ordinary runtime threads in a debug build. Keeping that payload in the watcher's
state SHALL remain subject to the ordinary 64-level state limit; reading a notice does not grant a state exception.

#### Scenario: Store and local typed watcher

- **WHEN** a depth-64 message fails under a watch on memory, SQLite or PostgreSQL
- **THEN** the store writes and claims its complete Failed notice
- **AND** a local typed watcher compares its payload through a probe without copying it into state and completes normally

#### Scenario: HTTP worker watcher

- **WHEN** that notice is leased over HTTP to a Worker hosting the typed watcher
- **THEN** the lease decodes, the handler sees the original payload and the notice turn completes normally

#### Scenario: Retaining the notice in an ordinary state

- **WHEN** a watcher instead copies the depth-65 notice into its own state and returns
- **THEN** the turn fails at state validation rather than at message decoding, with no new depth exemption
