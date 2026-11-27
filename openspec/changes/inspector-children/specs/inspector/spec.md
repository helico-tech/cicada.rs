## ADDED Requirements

### Requirement: Children in the Actors table

A row of the Actors table SHALL fold open to show its actor's children, read when it is opened, a page at a time per
parent. Each row SHALL have an identity by its path from the root, so one actor shown in two places has its own
selection and fold state in each, and a reload SHALL keep the open rows that still exist.

#### Scenario: Many children

- **WHEN** an actor has 150 children and the user opens its row and picks "more…"
- **THEN** its row shows 150 children, read in two pages by the parent's own cursor

#### Scenario: One actor in two places

- **WHEN** an actor is listed at the root and also as the child of another listed actor, and the user opens it at the
  root
- **THEN** only the root row is open, and selecting either row fills the pane with that actor

#### Scenario: A reload keeps open rows

- **WHEN** two rows are open, one of their actors is deleted, and the user presses ⌘R
- **THEN** the remaining row is open again after the reload and the deleted one is gone
