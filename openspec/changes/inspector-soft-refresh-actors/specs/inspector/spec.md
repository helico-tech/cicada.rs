## ADDED Requirements

### Requirement: A soft refresh of the Actors page

The Actors page SHALL offer a soft refresh that re-reads its root rows up to the old last address (at most 300) and the
first children page of at most 20 open rows, nearest the view first, keeps the list sorted and unique, keeps rows it did
not re-read marked as not refreshed, keeps the selection by path and the open rows, and re-reads the pane without
showing "Loading".

#### Scenario: An open row's child is deleted

- **WHEN** an open row on screen has three children, one is deleted, and a soft refresh runs
- **THEN** the deleted child disappears, the row stays open, and the selection stays where it was

#### Scenario: Many open rows

- **WHEN** 30 rows are open at one depth and a soft refresh runs
- **THEN** the first children pages of the 20 nearest rows are re-read, visible rows first, and the rest are marked as
  not refreshed

#### Scenario: A refreshed ancestor loses an open branch

- **WHEN** both an ancestor and a descendant are chosen for re-read and the ancestor's new children omit the path to
  the descendant
- **THEN** the ancestor's children are applied first and the descendant is pruned without a children read
- **AND** a selection under the pruned path is cleared with an empty pane

#### Scenario: The pane stays

- **WHEN** a soft refresh re-reads the selected actor's pane
- **THEN** the pane keeps its old content until the new reads answer, with no "Loading"
