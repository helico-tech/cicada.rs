## ADDED Requirements

### Requirement: Running Turns and Nodes pages

The inspector SHALL list the running turns a page at a time and the nodes, each with a read-only pane, SHALL show a
node's running turns in its pane, and SHALL show a node's health as the server computed it, without measuring ages on
the Mac's clock.

#### Scenario: A node's turns

- **WHEN** a node runs two turns and the user selects it on the Nodes page
- **THEN** the pane lists both turns, and Show Actor on one selects its actor on the Actors page

#### Scenario: A gone node

- **WHEN** the selected node's row has been removed from the node table
- **THEN** the pane says "not found"

#### Scenario: Duplicates across pages

- **WHEN** a turn's actor appears on two pages of Running Turns because the list changed between them
- **THEN** the table shows it once
