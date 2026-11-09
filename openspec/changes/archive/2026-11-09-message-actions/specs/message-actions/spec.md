## ADDED Requirements

### Requirement: Cancelling a pending message is a POST

`POST /messages/{id}/cancel` SHALL replace `DELETE /messages/{id}` with the same semantics: `204` when pending, `404`
unknown, `409` running or finished. A running turn is stopped with `turn/cancel`, not here.

#### Scenario: A running message

- **WHEN** a client cancels a message that is being handled
- **THEN** the answer is `409` and the turn keeps running

### Requirement: A failed or cancelled message can be retried

`POST /messages/{id}/retry` SHALL answer `202` with a new message: a new id and the original's target, type, payload,
correlation id, headers, retry and retention, `causedBy` the original. It SHALL answer `404` for an unknown, removed or
past-retention id (or `no-such-instance` when the target is gone) and `409 message-not-retryable` for a message that is
pending, running, done or purged.

#### Scenario: A failed message

- **WHEN** a client retries a message that ended `failed`
- **THEN** the answer is `202`, a new message with a new id is pending, and its `causedBy` is the original

#### Scenario: A done message

- **WHEN** a client retries a message that ended `done`
- **THEN** the answer is `409 message-not-retryable`

### Requirement: A pending message can be delivered now

`POST /messages/{id}/deliver-now` SHALL make a pending, not running message due at store now with a new `seq` and answer
`200` with the message, `404` when unknown, and `409` when running or finished.

#### Scenario: A delayed message

- **WHEN** a message is delayed by an hour and a client delivers it now
- **THEN** it becomes due at once and is handled
