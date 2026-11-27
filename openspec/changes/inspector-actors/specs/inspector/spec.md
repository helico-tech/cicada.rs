## ADDED Requirements

### Requirement: The Actors page

The inspector SHALL list the actors under a prefix a page at a time with their type, version, created time and status,
and SHALL show the selected actor's state, running turn, details, mailbox and watches in a pane, read only. A read
that no longer matches the current prefix, selection or context SHALL be dropped.

#### Scenario: Paging

- **WHEN** 250 actors exist and the user scrolls to the end of the table twice
- **THEN** the table holds 250 rows, read in three pages, with no page read twice

#### Scenario: A search drops older pages

- **WHEN** the user types a prefix while a page for the earlier prefix is still loading
- **THEN** that page is dropped and the table shows only actors under the new prefix

#### Scenario: The pane

- **WHEN** the user selects an actor with a running turn and two pending messages
- **THEN** the pane shows its state as JSON, the turn's message type and attempt, and two rows under Mailbox

#### Scenario: Back to an earlier selection

- **WHEN** the user selects actor A, then B, then A again before the first read of A answers
- **THEN** the pane shows only the last read of A
