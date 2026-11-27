## ADDED Requirements

### Requirement: GUI changes name their screens and boards

A change that adds or changes a screen SHALL name each such screen in its design with its mockup board or "no board",
SHALL carry a copy of each board in its folder with its source and date, and SHALL build a screen without a board only
when the owner agreed to that in the design.

#### Scenario: A screen without a board

- **WHEN** a change adds a sheet that the mockup has no board for
- **THEN** its design names the sheet with "no board", and it is built only after the owner has a board drawn or agrees
  to build it in the named board's style

### Requirement: GUI changes are built and reviewed against their boards

A GUI change SHALL list each departure from its boards, additions included, with its reason in its design's
"Departures from the board" for the owner to agree, and SHALL be built and code reviewed against the boards; an element
missing, changed or added without an agreed departure SHALL block landing.

#### Scenario: A screen with a board

- **WHEN** a change builds an inspector page that the mockup has a board for
- **THEN** the board is in the change's `mockup/` folder, the building task names it, and the code review compares the
  page's code with it in both directions

#### Scenario: A departure found while building

- **WHEN** the builder finds that a native control cannot look like the board
- **THEN** it reports the departure as a question for the owner instead of choosing another look

### Requirement: A changed board is settled before landing

Before a GUI change lands, the change loop SHALL read its boards' live versions again; when one changed after the copy,
the change SHALL NOT land until the owner accepts the copied version or the change is updated to the new copy and
reviewed against it.

#### Scenario: The owner edits a board during the build

- **WHEN** the live board differs from the copy in the change at landing
- **THEN** the change waits for the owner to accept the copy, or for a review against the new copy
