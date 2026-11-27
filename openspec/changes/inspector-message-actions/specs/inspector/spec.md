## ADDED Requirements

### Requirement: Message actions in the inspector

The message panes SHALL offer Retry for a failed or cancelled message, and Cancel and Deliver Now for a pending one that
is not running, by the state last read. Retry and Cancel SHALL ask first, Retry naming how often the message was retried
before, as last read. Refusals SHALL show as alerts with the server's message, and a success SHALL be followed by a
fresh read.

#### Scenario: Retry a failed message

- **WHEN** the operator retries a failed outcome that was never retried and confirms
- **THEN** the pane says it was retried with the new message id, and the page is read again

#### Scenario: Retry asks with the count

- **WHEN** the operator presses Retry on a failed outcome whose Caused list holds two messages
- **THEN** the sheet says it was retried 2 times before and that a retry may repeat external work

#### Scenario: Cancel a message that started running

- **WHEN** the operator cancels a message that the server is handling by then
- **THEN** the pane says it is being handled and was not cancelled

### Requirement: Actions on several messages

The Messages and Outcomes tables SHALL allow several rows to be selected; Outcomes SHALL offer to retry the selected
failed or cancelled rows and Messages to cancel the selected pending rows, at most 1,000 at once, asking first. A bulk
retry SHALL skip rows already retried, as just read, and a bulk call whose result is unknown SHALL NOT be sent again on
its own.

#### Scenario: Bulk retry skips retried rows

- **WHEN** the operator selects three failed outcomes, one of which was already retried, and presses Retry
- **THEN** the sheet offers to retry 2 of 3 and names the skipped one, and only those 2 are sent

#### Scenario: Bulk cancel

- **WHEN** the operator cancels five selected pending messages and one has started running by then
- **THEN** the page says 4 of 5 were cancelled

#### Scenario: Too many rows

- **WHEN** more than 1,000 eligible rows are selected
- **THEN** the bulk button is disabled and says at most 1,000

#### Scenario: A bulk call times out

- **WHEN** a bulk retry times out
- **THEN** the page says the result is unknown and asks to read the page again, and nothing is sent again by itself
