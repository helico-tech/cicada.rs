# messages-by-cause Specification

## Purpose
TBD - created by archiving change messages-by-cause. Update Purpose after archive.

## Requirements

### Requirement: Messages caused by a message are listed

`GET /messages/{id}/caused` SHALL list, one level down, the messages whose `causedBy` is `id`: pending and running
ones, and finished ones whose outcome is retained and visible. Each item SHALL have the form of `GET /messages/{id}`.
They SHALL be ordered by message id and paged by `cursor` and `limit`. When `id` itself is unknown or past retention the
answer SHALL be `404` with the problem type `no-such-message`.

#### Scenario: Children of a message

- **WHEN** the turn that handled a message sent three messages
- **THEN** the list holds those three, in message-id order, whatever their state

#### Scenario: One level only

- **WHEN** one of the children sent a message in turn
- **THEN** the grandchild is not in the list of the first message, but in the list of its parent

#### Scenario: A finished child

- **WHEN** a child is done and its outcome is retained
- **THEN** it is listed with the status `done`

#### Scenario: A removed record

- **WHEN** a child's retention policy removed its outcome, or its outcome is past retention
- **THEN** it is not listed

#### Scenario: An unknown message

- **WHEN** the id is unknown
- **THEN** the answer is `404 no-such-message`

### Requirement: The cause is stored with the outcome

A message's outcome SHALL keep the id of the message that caused it, in a column of its own with an index, and a pending
message's cause SHALL be indexed.

#### Scenario: Finishing a caused message

- **WHEN** a caused message is handled
- **THEN** the lookup by cause finds its outcome after it left the mailbox
