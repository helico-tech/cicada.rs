## ADDED Requirements

### Requirement: GUI changes follow their mockup board

A change that adds or changes a screen SHALL carry a copy of each mockup board it follows in its folder, SHALL list in
its design every departure from those boards with its reason for the owner to agree, and SHALL be built and code
reviewed against the boards; an element missing or changed without an agreed departure SHALL block landing.

#### Scenario: A screen with a board

- **WHEN** a change builds an inspector page that the mockup has a board for
- **THEN** the board is in the change's `mockup/` folder, the building task names it, and the code review compares the
  page's code with it

#### Scenario: A departure found while building

- **WHEN** the builder finds that a native control cannot look like the board
- **THEN** it reports the departure as a question for the owner instead of choosing another look
