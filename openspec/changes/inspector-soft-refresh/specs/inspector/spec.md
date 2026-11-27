## ADDED Requirements

### Requirement: A soft refresh that keeps the view

Each inspector page SHALL offer a soft refresh that re-reads the loaded rows within fixed limits (at most 300 root
rows, the children of at most 20 open rows, 4 requests at a time, 10 s), replaces rows within the re-read range, keeps
rows past it marked as not refreshed, and puts back the selection, the scroll position and the open rows, without
undoing a change the user made while it ran.

#### Scenario: A deleted row

- **WHEN** a soft refresh runs on Actors with 150 rows loaded and one of them has been deleted
- **THEN** that row disappears, the others keep their place, and the selection stays on its row

#### Scenario: Rows past the limit

- **WHEN** 450 rows are loaded and a soft refresh runs
- **THEN** the first 300 are re-read and the last 150 stay, marked as not refreshed

#### Scenario: The user selects during a refresh

- **WHEN** the user selects another row while a soft refresh is running
- **THEN** the new selection stays when the refresh ends
