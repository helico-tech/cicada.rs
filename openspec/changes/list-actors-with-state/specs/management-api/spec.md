## ADDED Requirements

### Requirement: A host lists actors with their state

ActorSystem SHALL offer list_actors_with_state returning Page<ActorSnapshot>. ActorClient SHALL offer the same
method returning PageView<ActorView> through GET /actors/states, served by both HTTP routers. Rows SHALL reuse the
complete single-actor snapshot and HTTP view without actor-type decoding. Existing list_actors and GET /actors SHALL
remain address-only and unchanged.

#### Scenario: A page holds complete stored snapshots

- **GIVEN** a/1 has state at version 2, a/2 is paused, and a/3 is a child of a/1
- **WHEN** a host lists actors with state under a/ with no intervening writes
- **THEN** the three rows are ordered by address and each equals the result of reading that actor separately
- **AND** the HTTP rows include address, type, state, version, createdAt, paused and parent, with null parent for
  independent actors; Rust snapshots have the corresponding fields except type, which is part of their address

#### Scenario: A node need not host the actor type

- **WHEN** a node that does not host the listed actor types serves their state listing
- **THEN** it returns their stored JSON and metadata without decoding them through an actor or checking a schema

#### Scenario: Both routers expose the read

- **WHEN** GET /actors/states is sent through either actor_api or actor_api_read
- **THEN** it returns the state page with the host's enclosing layers applied as for other reads

#### Scenario: The address listing stays compatible

- **WHEN** an existing client calls list_actors or GET /actors
- **THEN** it receives the same address-only page and cursors, and the listActors OpenAPI operation has no response union

#### Scenario: The fixed route does not hide the states actor type

- **WHEN** GET /actors/states and GET /actors/states/one are requested
- **THEN** the first lists actor states and the second reads the actor at states/one

#### Scenario: The generated contract and client describe the new route

- **WHEN** a client reads either router's OpenAPI document
- **THEN** GET /actors/states has operationId listActorStates, the listing query parameters and ActorViewPage response,
  and its errors are application/problem+json
- **AND** the typed client's new method uses this route with no per-item fallback against an older server

### Requirement: State pages reuse address keyset paging

State listings SHALL reuse the address listing's order, literal canonical-address prefix and keyset cursors, with
no new cursor format or validation. Core SHALL reject zero without a new upper cap; HTTP limits SHALL be 1..1000, default 100; next SHALL exist only with
a lookahead row. The HTTP query parser SHALL retain existing listing rules and reject invalid limits with 400.

#### Scenario: Pages have the same boundaries as address-only pages

- **GIVEN** three actors under a/ and no concurrent changes
- **WHEN** both listing forms are read with limit 2 and then their returned next cursor
- **THEN** both first pages name the same two addresses and have the same next cursor
- **AND** both second pages name the third address and have next null

#### Scenario: No matching rows

- **WHEN** the prefix matches no actors or the cursor lies beyond the final matching address
- **THEN** HTTP returns 200 with {"items":[],"next":null}

#### Scenario: A cursor before the prefix

- **WHEN** a cursor sorts before the requested prefix
- **THEN** listing starts at the prefix, matching the address-only listing

#### Scenario: Prefixes are literal address prefixes

- **GIVEN** actors under a/ and ab/ and keys whose canonical addresses contain percent signs and underscores
- **WHEN** their literal prefixes are listed, including an empty prefix
- **THEN** matching is identical to the address-only listing with no wildcard expansion
- **AND** prefix a matches both types while a/ matches only the a type

#### Scenario: Core keeps its existing limit rules

- **WHEN** list_actors_with_state is called directly with limit 0 or 1001
- **THEN** zero is refused as for list_actors, while 1001 is accepted with no HTTP-sized cap added to core

#### Scenario: Limits and duplicate query values follow existing listings

- **WHEN** limit is absent, 1, 1000, 0, 1001, negative or nonnumeric
- **THEN** absence selects 100, 1 and 1000 are accepted, and the others return 400
- **AND** limit=1&limit=2 selects 1, unknown query keys are ignored and an empty cursor is treated as absent

### Requirement: Each state page is one committed store read

A state-page read attempt SHALL use one actor SELECT in a SQL read transaction, or one memory-store lock, without
per-actor reads. Rows SHALL share one committed read snapshot, each state paired with its version. Existing store
retries remain permitted. There SHALL be no snapshot across pages; version retains its meaning of state writes in
one actor life, not a global change sequence.

#### Scenario: A running handler has not committed

- **WHEN** a page is read while a handler has changed its local state but has not committed
- **THEN** the row holds the preceding committed state and version
- **AND** a page read after its successful commit sees the committed state and version together

#### Scenario: A turn changes multiple listed rows atomically

- **WHEN** a turn commits its own new state and creates a child while a state page containing both addresses is read
- **THEN** the page observes the committed effects together or neither, never a mix of per-row reads across that commit

#### Scenario: Writes between pages

- **WHEN** an actor is inserted before the first page's cursor, an unvisited actor is deleted, and another unvisited
  actor's state is committed before the next page is read
- **THEN** the next page misses the earlier insertion, omits the deletion and shows the later committed version
- **AND** deleting and recreating an actor does not turn its state version into a durable identity or global watermark

### Requirement: State listing preserves existing read and error rules

State listings SHALL preserve existing read/error rules: no new depth check, truncation or byte cap, including for
legacy values (ADR 0029). A malformed fetched record, including decoded lookahead, SHALL fail the whole page. HTTP
SHALL map corrupt data to 500 and store unavailability to 503. Current 64-level states SHALL fit in the 67-level page.

#### Scenario: A legacy deep state remains readable

- **GIVEN** a 100-level state was stored before the write-depth limit
- **WHEN** it is listed with state
- **THEN** it is returned as stored without a new depth error or truncation
- **AND** the guide warns that still deeper legacy values may exceed a client's parser limit for the entire page

#### Scenario: A bad row or unavailable store fails the page

- **WHEN** a fetched row cannot be decoded by the store or the store is unavailable
- **THEN** no partial page is returned, and HTTP answers the existing 500 or 503 problem response respectively

#### Scenario: A maximum-depth current state fits the view

- **WHEN** a stored 64-level state is listed over HTTP and read by the typed client
- **THEN** it survives round-trip in the 67-level page without changing its value
