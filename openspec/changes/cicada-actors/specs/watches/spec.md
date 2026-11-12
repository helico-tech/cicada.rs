# watches

## MODIFIED Requirements

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

## ADDED Requirements

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
