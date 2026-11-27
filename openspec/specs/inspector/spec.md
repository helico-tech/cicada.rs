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

### Requirement: The inspector inside the tray

With its feature `inspector`, the tray SHALL open the inspector in its own process on its connected context, else on
the target `cicada` would use, SHALL bring an open inspector to the front without changing its context, and SHALL close
the inspector, with its pending refreshes, on every quit path before it quits, and not open it while quitting.

#### Scenario: Open twice

- **WHEN** the tray is online with `prod` and the user picks Open Inspector…, switches the inspector to `staging`, and
  picks Open Inspector… again
- **THEN** one inspector window shows `staging` and comes to the front

#### Scenario: Quit with the inspector open

- **WHEN** the inspector is open and refreshing, and the user quits the tray
- **THEN** the inspector closes, its refresh is dropped, and the tray quits as before

#### Scenario: Without the feature

- **WHEN** the tray is built without the feature `inspector`
- **THEN** the menu has no Open Inspector… item

### Requirement: The Actors page

The inspector SHALL list the actors under a prefix a page at a time with their type, version, created time and status,
and SHALL show the selected actor's state, running turn, details, mailbox and watches in a pane, read only. A read
that no longer matches the current prefix, selection or context SHALL be dropped.

#### Scenario: Paging

- **WHEN** 250 actors exist and the user scrolls to the end of the table twice
- **THEN** the table holds 250 rows, read in three pages, with no page read twice

#### Scenario: A search drops older pages

- **WHEN** the user types a prefix while a page for the earlier prefix is still loading
- **THEN** that page is dropped at the first key press and the table shows only actors under the new prefix

#### Scenario: The pane

- **WHEN** the user selects an actor with a running turn and two pending messages
- **THEN** the pane shows its state as JSON, the turn's message type and attempt from the mailbox's first 50 rows,
  and the two pending messages under Mailbox

#### Scenario: Back to an earlier selection

- **WHEN** the user selects actor A, then B, then A again before the first read of A answers
- **THEN** the pane shows only the last read of A

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

### Requirement: The Messages page and Show Actor

The inspector SHALL list the pending messages under a prefix a page at a time, each message once, with a read-only pane
for the selected one, and SHALL go from a message to its actor on the Actors page, recorded in back and forward.

#### Scenario: A pending message

- **WHEN** the user selects a pending message on the Messages page
- **THEN** the pane shows its envelope and its payload as JSON

#### Scenario: Duplicates across pages

- **WHEN** a message is retried between two pages of Messages so that both pages hold it
- **THEN** the table shows it once

#### Scenario: Show Actor

- **WHEN** the user picks Show Actor for a message, and later goes back
- **THEN** the Actors page selects exactly that message's actor, and back returns to the message

### Requirement: Types and Watches pages

The inspector SHALL list the registered types with a pane that shows one chosen schema at a time, and the watches a
page at a time with a pane and Show Actor, both read only, with local filters where the API has none. While a
watches filter is typed, it SHALL load further pages only when the user asks.

#### Scenario: A type's schemas

- **WHEN** the user selects a type with two message types and picks the second message in the pane's pop-up
- **THEN** the pane shows that message's schema, and only it was formatted

#### Scenario: A filter that hides every loaded watch

- **WHEN** the loaded watches hold no watcher or target under the typed prefix and the server has more
- **THEN** the table is empty, no page is read by itself, the footer says the filter covers the loaded rows, and Load
  more is offered

#### Scenario: From a watch to its watcher

- **WHEN** the user picks Show Actor for a watch's watcher
- **THEN** the Actors page selects exactly that actor, and back returns to the watch

#### Scenario: A failed read

- **WHEN** the `types` read fails, or a watches page fails while the filter leaves no row visible
- **THEN** the footer shows the error with Retry, and no stale list of types is kept

### Requirement: The Outcomes page

The inspector SHALL list finished messages by status (failed by default) and prefix a page at a time, show the failed
count in the sidebar, and show the selected outcome's message, error and payload in a read-only pane with its cause
chain one step at a time, recorded in back and forward.

#### Scenario: Failed outcomes

- **WHEN** two messages failed and one is done, and the user opens Outcomes
- **THEN** the table lists the two failed ones and the sidebar shows 2

#### Scenario: Following the cause

- **WHEN** the user selects a failed outcome whose message was sent by another turn, follows Caused by, and goes back
- **THEN** the pane shows the causing message, and back returns to the failed outcome

#### Scenario: Past retention

- **WHEN** the causing message is past retention
- **THEN** following Caused by shows "not found" in the pane

#### Scenario: Back to an outcome no longer listed

- **WHEN** the user follows Caused by from a failed outcome, switches the status to Done, and goes back
- **THEN** the pane shows the failed outcome as it was read
