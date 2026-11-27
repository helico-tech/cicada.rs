## ADDED Requirements

### Requirement: Messages, Types and Watches pages

The inspector SHALL list the pending messages under a prefix, the registered types and the watches, each with a
read-only pane for the selected row, and SHALL let the user go from a message or a watch to its actor.

#### Scenario: A pending message

- **WHEN** the user selects a pending message on the Messages page
- **THEN** the pane shows its envelope and its payload as JSON, and Show Actor selects its actor on the Actors page

#### Scenario: A type's schemas

- **WHEN** the user selects a type with two messages on the Types page
- **THEN** the pane shows its state schema and both message schemas

#### Scenario: Duplicates across pages

- **WHEN** a message is retried between two pages of Messages so that both pages hold it
- **THEN** the table shows it once
