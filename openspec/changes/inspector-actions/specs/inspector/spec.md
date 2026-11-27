## MODIFIED Requirements

### Requirement: The Actors page

The inspector SHALL list the actors under a prefix a page at a time with their type, version, created time and status,
and SHALL show the selected actor's state, running turn, details, mailbox and watches in a pane, read only except for
the actor actions of "Actor actions in the inspector". A read that no longer matches the current prefix, selection or
context SHALL be dropped.

#### Scenario: Paging

- **WHEN** 250 actors exist and the user scrolls to the end of the table twice
- **THEN** the table holds 250 rows, read in three pages, with no page read twice

#### Scenario: A search drops older pages

- **WHEN** the user types a prefix while a page for the earlier prefix is still loading
- **THEN** that page is dropped at the first key press and the table shows only actors under the new prefix

#### Scenario: The pane

- **WHEN** the user selects an actor with a running turn and two pending messages
- **THEN** the pane shows its state as JSON, the turn's message type and attempt from the mailbox's first 50 rows,
  and the two pending messages under Mailbox

#### Scenario: Back to an earlier selection

- **WHEN** the user selects actor A, then B, then A again before the first read of A answers
- **THEN** the pane shows only the last read of A

## ADDED Requirements

### Requirement: Actor actions in the inspector

The actor pane SHALL offer Pause or Resume, Cancel Turn while a turn runs, Purge Mailbox and Delete, through the same
API calls as `cicada`, one action at a time on the target its button was pressed for. Cancel Turn, Purge and Delete
SHALL ask first in a sheet that names what goes, with Cancel as the default for Return and Escape.

#### Scenario: Delete with descendants

- **WHEN** the user picks Delete… on an actor with two children and confirms
- **THEN** the actor and its children are deleted, its row and subtree disappear at once, and the pane says "not found"

#### Scenario: Return cancels

- **WHEN** the Delete sheet is open and the user presses Return
- **THEN** nothing is deleted

#### Scenario: The selection changed

- **WHEN** the user opens the Purge sheet for actor A, the selection moves to B, and the user confirms
- **THEN** nothing is sent and the pane says the selection changed

### Requirement: Results of actor actions

An absent actor SHALL show local text and another refusal an alert with the server's message, leaving the pane as it
was; a successful write SHALL be followed by a fresh read, also with live updates paused.

#### Scenario: An actor deleted meanwhile

- **WHEN** the user purges the mailbox of an actor that was deleted meanwhile
- **THEN** the pane says "actor not found" and nothing else changes

#### Scenario: A pause while live updates are paused

- **WHEN** live updates are paused and the user pauses an actor
- **THEN** the pane shows the actor paused after a fresh read
