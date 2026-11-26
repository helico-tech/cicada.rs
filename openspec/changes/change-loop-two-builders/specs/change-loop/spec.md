## ADDED Requirements

### Requirement: Two builders when their changes share no crate

`scripts/status.py` SHALL name up to two changes to build: the first ready change in the order, and the next ready
change whose `Crates:` line names no crate of the first's; a ready change without that line SHALL be built alone.

#### Scenario: A placement change and a tray change

- **WHEN** a ready change names `Crates: cicada-actors` and the next ready one `Crates: cicada-tray`
- **THEN** the `build:` line names both

#### Scenario: A shared crate

- **WHEN** both ready changes name `cicada-actors`
- **THEN** the `build:` line names only the first

### Requirement: A change is reviewed by another model than its builder's

`scripts/review.py request` SHALL record who built a change, and `take` SHALL refuse a reviewer of the same model
family.

#### Scenario: A Claude reviewer for a Claude build

- **WHEN** a change built by Claude is taken for review by a Claude reviewer
- **THEN** `take` refuses it, and a GPT reviewer can take it
