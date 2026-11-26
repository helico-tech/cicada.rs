## ADDED Requirements

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
