# change-loop Specification

## Purpose
TBD - created by archiving change change-loop-two-builders. Update Purpose after archive.

## Requirements

### Requirement: Two builders when their changes share no crate

`scripts/status.py` SHALL name up to two changes to build: those in build, and for a free slot the first ready change
in the order whose `Crates:` line meets no crate of a change in build; a ready change without that line SHALL be built
only when no other build runs, and two changes in build that share a crate SHALL be listed as a problem.

#### Scenario: A placement change and a tray change

- **WHEN** a change in build names `Crates: cicada-actors` and the next ready one `Crates: cicada-tray`
- **THEN** the `build:` line names both

#### Scenario: A shared crate

- **WHEN** the change in build and the next ready change both name `cicada-actors`, and a later ready change names
  `Crates: none`
- **THEN** the `build:` line names the change in build and the later one

#### Scenario: A change without a Crates line

- **WHEN** the next ready change has no `Crates:` line and another change is in build
- **THEN** it is not started until that build ends

#### Scenario: Landing out of order

- **WHEN** the second of two built changes lands first and is archived
- **THEN** `status.py` still shows the open changes as one line and reports no problem

### Requirement: A change is reviewed by another model than its builder's

`scripts/review.py request` SHALL record who built a change, `take ID` SHALL refuse a reviewer of the builder's model
family, and `take` without an id SHALL skip such requests; a same-family review taken because Codex is unavailable
SHALL be recorded as such.

#### Scenario: A Claude reviewer for a Claude build

- **WHEN** a change built by Claude is taken for review by a Claude reviewer
- **THEN** `take` refuses it, and a GPT reviewer can take it

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

- **WHEN** a change builds a tray screen that the mockup has a board for
- **THEN** the board is in the change's `mockup/` folder, the building task names it, and the code review compares the
  screen's code with it in both directions

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
