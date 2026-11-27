## ADDED Requirements

### Requirement: GUI changes follow their mockup board

A change that adds or changes a screen SHALL name each such screen in its design with its mockup board or "no board",
SHALL carry a copy of each board in its folder with its source and date, SHALL build a screen without a board only when
the owner agreed to that in the design, SHALL list every departure from a board with its reason for the owner to agree,
and SHALL be built and code reviewed against the boards; an element missing or changed without an agreed departure SHALL
block landing.

#### Scenario: A screen with a board

- **WHEN** a change builds an inspector page that the mockup has a board for
- **THEN** the board is in the change's `mockup/` folder, the building task names it, and the code review compares the
  page's code with it

#### Scenario: A departure found while building

- **WHEN** the builder finds that a native control cannot look like the board
- **THEN** it reports the departure as a question for the owner instead of choosing another look

#### Scenario: A screen without a board

- **WHEN** a change adds a sheet that the mockup has no board for
- **THEN** its design names the sheet with "no board", and it is built only after the owner has a board drawn or agrees
  to build it in the named board's style
