## ADDED Requirements

### Requirement: Actor actions in the inspector

The actor pane SHALL offer Pause or Resume, Cancel Turn while a turn runs, Purge Mailbox and Delete, through the same
API calls as `cicada`; Purge and Delete SHALL ask first in a sheet that names what goes, with cancel as the default; a
refusal of the API SHALL show an alert with the server's message and leave the pane as it was.

#### Scenario: Delete with descendants

- **WHEN** the user picks Delete… on an actor with two children and confirms
- **THEN** the actor and its children are deleted, its row disappears, and the pane says "not found"

#### Scenario: A refused action

- **WHEN** the user purges the mailbox of an actor that was deleted meanwhile
- **THEN** an alert shows the server's message and the pane is unchanged

#### Scenario: Return cancels

- **WHEN** the Delete sheet is open and the user presses Return
- **THEN** nothing is deleted
