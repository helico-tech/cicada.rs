## ADDED Requirements

### Requirement: The Messages page and Show Actor

The inspector SHALL list the pending messages under a prefix a page at a time, each message once, with a read-only pane
for the selected one, and SHALL go from a message to its actor on the Actors page, recorded in back and forward.

#### Scenario: A pending message

- **WHEN** the user selects a pending message on the Messages page
- **THEN** the pane shows its envelope and its payload as JSON

#### Scenario: Duplicates across pages

- **WHEN** a message is retried between two pages of Messages so that both pages hold it
- **THEN** the table shows it once

#### Scenario: Show Actor

- **WHEN** the user picks Show Actor for a message, and later goes back
- **THEN** the Actors page selects exactly that message's actor, and back returns to the message
