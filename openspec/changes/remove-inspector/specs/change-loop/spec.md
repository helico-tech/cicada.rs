## MODIFIED Requirements

### Requirement: GUI changes are built and reviewed against their boards

A GUI change SHALL list each departure from its boards, additions included, with its reason in its design's
"Departures from the board" for the owner to agree, and SHALL be built and code reviewed against the boards; an element
missing, changed or added without an agreed departure SHALL block landing.

#### Scenario: A screen with a board

- **WHEN** a change builds a tray screen that the mockup has a board for
- **THEN** the board is in the change's `mockup/` folder, the building task names it, and the code review compares the
  screen's code with it in both directions

#### Scenario: A departure found while building

- **WHEN** the builder finds that a native control cannot look like the board
- **THEN** it reports the departure as a question for the owner instead of choosing another look
