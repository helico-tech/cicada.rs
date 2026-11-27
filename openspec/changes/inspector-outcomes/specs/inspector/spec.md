## ADDED Requirements

### Requirement: The Outcomes page

The inspector SHALL list finished messages by status (failed by default) and prefix a page at a time, show the failed
count in the sidebar, and show the selected outcome's message, error and payload in a read-only pane with its cause
chain one step at a time, recorded in back and forward.

#### Scenario: Failed outcomes

- **WHEN** two messages failed and one is done, and the user opens Outcomes
- **THEN** the table lists the two failed ones and the sidebar shows 2

#### Scenario: Following the cause

- **WHEN** the user selects a failed outcome whose message was sent by another turn, follows Caused by, and goes back
- **THEN** the pane shows the causing message, and back returns to the failed outcome

#### Scenario: Past retention

- **WHEN** the causing message is past retention
- **THEN** following Caused by shows "not found" in the pane
