## MODIFIED Requirements

### Requirement: The Messages page and Show Actor

The inspector SHALL list the pending messages under a prefix a page at a time, each message once, with several rows
selectable and a pane for the clicked one, read only except for the message actions of "Message actions in the
inspector" and "Actions on several messages", and SHALL go from a message to its actor on the Actors page, recorded in
back and forward.

#### Scenario: A pending message

- **WHEN** the user selects a pending message on the Messages page
- **THEN** the pane shows its envelope and its payload as JSON

#### Scenario: Duplicates across pages

- **WHEN** a message is retried between two pages of Messages so that both pages hold it
- **THEN** the table shows it once

#### Scenario: Show Actor

- **WHEN** the user picks Show Actor for a message, and later goes back
- **THEN** the Actors page selects exactly that message's actor, and back returns to the message

### Requirement: The Outcomes page

The inspector SHALL list finished messages by status (failed by default) and prefix a page at a time, show the failed
count in the sidebar, allow several rows to be selected, and show the clicked outcome's message, error and payload in a
pane, read only except for the message actions of "Message actions in the inspector" and "Actions on several messages",
with its cause chain one step at a time, recorded in back and forward.

#### Scenario: Failed outcomes

- **WHEN** two messages failed and one is done, and the user opens Outcomes
- **THEN** the table lists the two failed ones and the sidebar shows 2

#### Scenario: Following the cause

- **WHEN** the user selects a failed outcome whose message was sent by another turn, follows Caused by, and goes back
- **THEN** the pane shows the causing message, and back returns to the failed outcome

#### Scenario: Past retention

- **WHEN** the causing message is past retention
- **THEN** following Caused by shows "not found" in the pane

#### Scenario: Back to an outcome no longer listed

- **WHEN** the user follows Caused by from a failed outcome, switches the status to Done, and goes back
- **THEN** the pane shows the failed outcome as it was read

## ADDED Requirements

### Requirement: Message actions in the inspector

The message panes SHALL offer Retry for a failed or cancelled message, and Cancel and Deliver Now for a pending message
(on the Messages pane for every row, as its listing does not say which one runs), by the state last read. Retry and
Cancel SHALL ask first, Retry naming the message's retries still kept, as last read. Refusals SHALL show as alerts with
the server's message, and a success SHALL be followed by a fresh read.

#### Scenario: Retry a failed message

- **WHEN** the operator retries a failed outcome that was never retried and confirms
- **THEN** the pane says it was retried with the new message id, and the page is read again

#### Scenario: Retry asks with the count

- **WHEN** the operator presses Retry on a failed outcome whose Caused list holds two messages
- **THEN** the sheet says 2 retries are still kept and that a retry may repeat external work

#### Scenario: Cancel a message that started running

- **WHEN** the operator cancels a message that the server is handling by then
- **THEN** the pane says it is being handled and was not cancelled

#### Scenario: A cancelled message past the refreshed range

- **WHEN** the operator cancels a Messages row that lies past the soft refresh's range, with Live paused
- **THEN** the pane shows the message as cancelled with no buttons, whichever of the refresh and the message's own read
  answers first

### Requirement: Actions on several messages

The Messages and Outcomes tables SHALL allow several rows to be selected; Outcomes SHALL offer to retry the selected
failed or cancelled rows and Messages to cancel the selected rows, at most 1,000 at once, asking first, sent in chunks
of 100. A bulk retry SHALL skip rows with a retry still kept, as just read, and a chunk whose result is unknown SHALL
stop the run and SHALL NOT be sent again on its own.

#### Scenario: Bulk retry skips retried rows

- **WHEN** the operator selects three failed outcomes, one of which was already retried, and presses Retry
- **THEN** the sheet offers to retry 2 of 3 with 1 already retried skipped, and only those 2 are sent

#### Scenario: Bulk cancel

- **WHEN** the operator cancels five selected pending messages and one has started running by then
- **THEN** the page says 4 of 5 were cancelled and 1 was not eligible by then

#### Scenario: Too many rows

- **WHEN** more than 1,000 eligible rows are selected
- **THEN** the bulk button is disabled and says at most 1,000

#### Scenario: A bulk call times out

- **WHEN** the second chunk of a bulk retry times out
- **THEN** the page says how many the first chunk retried, that the second chunk's result is unknown and the rest were
  not sent, and nothing is sent again by itself
