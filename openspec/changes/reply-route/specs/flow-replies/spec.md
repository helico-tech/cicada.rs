## ADDED Requirements

### Requirement: One reply route serves flow and actor callers

`ReplyRoute<T>` SHALL be either the flow form, holding a `ReplyTo<T>`, or the actor form, holding an actor address
and an optional correlation id. Its `send` SHALL send `FlowMsg::Reply` exactly as `ReplyTo::send` for the flow form.

#### Scenario: A flow caller

- **WHEN** a flow's `step.request` builds its message with `reply: r.into()`, and the target sends its reply through
  the route
- **THEN** the flow's `step.wait` for that request returns the value

### Requirement: The actor form sends a message with the caller's correlation

For the actor form, `send` SHALL send the value as a message to the address, with the same type-name checks as
`Turn::send_to`, the route's correlation id when set and else the turn's, no retry policy and the ordinary headers and
retention. A value that does not encode as a message SHALL return `TurnError::Invalid` and buffer nothing.

#### Scenario: An actor caller

- **WHEN** an actor sends a request with an actor route to itself with correlation id `c1`, and the target keeps the
  route and sends its reply in a later turn
- **THEN** the caller handles the reply as a message of its own family, with correlation id `c1`

#### Scenario: A value that is not a message

- **WHEN** a target sends `5` through an actor form
- **THEN** `send` returns `Invalid` and the turn buffers no message

#### Scenario: A caller whose family lacks the reply

- **WHEN** a target sends a reply through an actor form whose address belongs to a type without that message name
- **THEN** the send fails with `SendRejected` in the target's turn

### Requirement: The reply route's wire is decided by its key

The flow form SHALL encode as `{"run", "request"}`, the wire of `ReplyTo`, and the actor form as `{"actor"}` with an optional `"correlation"`. An object
with `actor` SHALL decode as the actor form and one without it as the flow form; an object with `actor` and also `run`
or `request` SHALL fail to decode. The schema SHALL be the same for every `T`.

#### Scenario: A saved ReplyTo

- **WHEN** a message saved with a `ReplyTo<T>` field is decoded with that field as `ReplyRoute<T>`
- **THEN** it decodes as the flow form with the same run and request

#### Scenario: Both forms in one object

- **WHEN** `{"actor": "repos/a", "run": "build/b1", "request": "m1"}` is decoded as a `ReplyRoute<T>`
- **THEN** decoding fails
