## ADDED Requirements

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
