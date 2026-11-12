# turn-observer

## ADDED Requirements

### Requirement: A node tells its observers when a turn starts and ends

A node SHALL call `turn_started` of each observer once for every turn it runs, after the claim and before it decodes
or runs anything, with the actor's address, the message as claimed and the store time of the claim. When the commit
of the turn succeeds, the node SHALL call `turn_ended` on the value that call returned, with the outcome that was
committed and the messages the commit sent. A claim that the claim limit fails is the exception: the spec
`claim-limit` says when its observers are told.

#### Scenario: A handler returns

- **WHEN** a node with an observer runs a turn whose handler returns and sends two messages
- **THEN** the observer is told the start, with the message and an `attempt` of 1, and then the end, with the outcome
  `done` and the two messages

#### Scenario: A handler fails and is retried

- **WHEN** a handler fails for a message whose retry policy has attempts left
- **THEN** the observer is told the end of that turn with a reschedule that holds the error, and the next attempt is
  a new start with an `attempt` of 2

#### Scenario: A handler fails for good

- **WHEN** a handler fails for a message without a retry policy
- **THEN** the observer is told the end with the outcome `failed` and the error

#### Scenario: A message that cannot be decoded

- **WHEN** a node claims a message whose payload does not fit its type
- **THEN** the observer is told the start and the end, with the outcome `failed`

#### Scenario: State that cannot be decoded

- **WHEN** a node claims a message for an actor whose state it cannot decode
- **THEN** the observer is told the start and the end, with the outcome `failed`

#### Scenario: A turn is cancelled

- **WHEN** an operator cancels a turn while its handler runs
- **THEN** the observer is told the end with the outcome `cancelled`

#### Scenario: A commit is rejected

- **WHEN** a handler returns after a send to an actor that does not exist
- **THEN** the observer is told the end once, with the outcome `failed` that the node committed in its place, and no
  message sent

### Requirement: A turn without a commit that its node saw succeed is dropped, not ended

A node SHALL NOT call `turn_ended` for a turn of which it saw no commit succeed. It SHALL drop the value that
`turn_started` returned, which is how an observer learns of it. A later claim of the same message SHALL be a new
start.

#### Scenario: A lease is lost

- **WHEN** a turn loses its lease while its handler runs, and another claim later handles the message
- **THEN** the value of the first turn is dropped without `turn_ended`, and the observer is told a second start, with
  an `attempt` of 2, and its end

#### Scenario: A node stops in the middle of a turn

- **WHEN** a node stops and gives back the lease of a turn whose handler has not ended
- **THEN** the value of that turn is dropped without `turn_ended`

#### Scenario: The actor is deleted while its turn runs

- **WHEN** an actor is deleted from outside while its handler runs
- **THEN** the value of that turn is dropped without `turn_ended`

#### Scenario: A commit whose answer is lost

- **WHEN** the store applies the commit of a turn, the node gets no answer, and its next try is answered that the
  lease is no longer held
- **THEN** the value of that turn is dropped without `turn_ended`, although the message has its outcome

### Requirement: An observer has no say in a turn

An observer SHALL NOT be able to refuse a turn or to change what it commits, other than by the headers of its
sends. A panic in an observer SHALL be caught and SHALL change nothing else: the turn runs and commits as it would
without that observer, the headers that observer added in that call are not used, and the other observers are
called.

#### Scenario: An observer panics at the start

- **WHEN** a node has two observers, the first panics in `turn_started` after it added a header, and the handler
  sends a message
- **THEN** the handler runs, the message ends `done`, the sent message has no header of the first observer, and the
  second observer is told the start and the end

#### Scenario: An observer panics at the end

- **WHEN** the first of two observers panics in `turn_ended`
- **THEN** the outcome of the message stands and the second observer is told the end

#### Scenario: An observer panics when its value is dropped

- **WHEN** a turn loses its lease and the drop of an observer's value panics
- **THEN** the node goes on: the values of the other observers are dropped, and the next claim of the message runs

### Requirement: Several observers are called in the order they were added

A node SHALL accept any number of observers and SHALL call them in the order they were added, at the start of a
turn and at its end.

#### Scenario: Two observers

- **WHEN** a node was built with observer A and then observer B, and runs a turn
- **THEN** the calls are: start of A, start of B, end of A, end of B

### Requirement: An observer can add headers to the sends of a turn

In `turn_started` an observer SHALL be able to add headers, and every message that the handler of that turn sends
SHALL carry them. A header that a send sets itself SHALL keep the send's value. Of two observers that add a header
of the same name, the one added later SHALL win. A notification of a watch SHALL carry none of them. A header of an
observer SHALL never fail a send: one that the limits of a message refuse, or that does not fit into a message any
more, SHALL be left out.

#### Scenario: Headers reach every send

- **WHEN** an observer adds the header `trace` and the handler sends two messages
- **THEN** both messages have the header `trace` with the observer's value

#### Scenario: A send's own header wins

- **WHEN** an observer adds the header `trace` and the handler sends a message that sets `trace` itself
- **THEN** the message has the value the send set

#### Scenario: Two observers add the same header

- **WHEN** observer A and then observer B were added, both add the header `trace`, and the handler sends a message
- **THEN** the message has the value of B

#### Scenario: A header that is too long

- **WHEN** an observer adds a header with a value of 5,000 characters and the handler sends a message
- **THEN** the send succeeds and the message does not have that header

#### Scenario: A header that a message refuses

- **WHEN** an observer adds a header whose value holds a NUL character, and one with a name of 300 characters, and
  the handler sends a message
- **THEN** the send succeeds and the message has neither header

#### Scenario: A message that is full

- **WHEN** an observer adds two headers and the handler sends a message with 63 headers of its own
- **THEN** the send succeeds with 64 headers: its own and the observer's header whose name sorts first

#### Scenario: A notification has no observer headers

- **WHEN** an observer adds the header `trace`, and a turn of a watched actor fails
- **THEN** the `Failed` in the watcher's mailbox has no header `trace`

### Requirement: The telemetry of `cicada-otel` is an observer

`ActorTelemetry` SHALL be a `TurnObserver`. It SHALL give every turn one span, from its claim to its commit, with
the outcome as `cicada.outcome`, and the messages a turn sends SHALL carry the trace context of its span. A turn
that ends `failed` or is rescheduled SHALL have an `exception` event with the error. A turn of which the node saw
no commit succeed SHALL end its span with the outcome `abandoned`.

#### Scenario: A span per turn

- **WHEN** a node with `ActorTelemetry` runs a turn that sends a message, and then the turn of that message
- **THEN** there are two spans, the first with `cicada.outcome` `done`, and the second is a child of the first

#### Scenario: A handler fails and is retried

- **WHEN** a handler fails and its message is retried
- **THEN** the span of that turn has an `exception` event with the error and `cicada.outcome` `rescheduled`

#### Scenario: State that cannot be decoded

- **WHEN** a node with `ActorTelemetry` claims a message for an actor whose state it cannot decode
- **THEN** the turn has a span with `cicada.outcome` `failed`, the error status and an `exception` event

#### Scenario: A turn that commits nothing

- **WHEN** a turn loses its lease
- **THEN** its span ends with `cicada.outcome` `abandoned` and no error status
