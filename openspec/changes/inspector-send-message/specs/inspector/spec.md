## ADDED Requirements

### Requirement: Sending a message from the inspector

The actor pane SHALL offer Send Message, a sheet with the actor's type's message types and a JSON payload checked
locally, by the server's own validator settings, against the chosen type's schema before Send is enabled, keeping one
message id per payload so a repeated Send does not deliver twice; the server's refusal SHALL show in the sheet with the
payload kept, and a success SHALL be followed by a fresh read.

#### Scenario: A payload that fails the schema

- **WHEN** the operator types a payload without a property the schema requires
- **THEN** the sheet says the payload does not match the schema and Send stays disabled

#### Scenario: A sent message

- **WHEN** the operator sends a valid payload
- **THEN** the sheet closes, the pane says the message was sent, and the pane is read again

#### Scenario: The server refuses

- **WHEN** the server answers 422 for a payload the local check let through
- **THEN** the sheet stays open with the payload and shows the server's message

#### Scenario: Send again after a timeout

- **WHEN** a send times out and the operator presses Send again without changing the payload
- **THEN** the same message id is sent, and the server's answer that it already has it shows as "already sent"
