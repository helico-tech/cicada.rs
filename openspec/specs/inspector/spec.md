# inspector Specification

## Purpose
TBD - created by archiving change inspector-window. Update Purpose after archive.

## Requirements

### Requirement: The inspector's Overview

The inspector SHALL show, for a context, the store's counts, the health, the nodes, the running turns and the types
with their actor counts, gathered through the HTTP API, and SHALL show a failed call on its own part while the others
still show.

#### Scenario: An Overview

- **WHEN** a server has two `counter` actors and one `sync` actor, one pending message and one failed outcome, and the
  inspector refreshes
- **THEN** the Overview shows 3 actors, 1 pending, 1 failed, the health, and `counter` with 2 actors and `sync` with 1

#### Scenario: One route fails

- **WHEN** `/nodes` answers an error and the other routes answer
- **THEN** the nodes part shows the error and the other parts show their numbers

#### Scenario: An older server

- **WHEN** `/stats` answers without `failed`
- **THEN** the failed tile shows "—"

### Requirement: Navigation

The sidebar selection SHALL keep a back and forward history.

#### Scenario: Back and forward

- **WHEN** the user selects Overview, then Nodes, then goes back, then selects Types
- **THEN** back leads to Overview and forward is empty

### Requirement: A bounded refresh

A refresh SHALL count the actors of at most 50 types, with at most 8 calls at a time and a deadline of 10 seconds, and
SHALL show a type it did not count as not counted, not as failed.

#### Scenario: Many types

- **WHEN** the server registers 60 types and the inspector refreshes
- **THEN** 50 types show their actor counts and 10 show as not counted
