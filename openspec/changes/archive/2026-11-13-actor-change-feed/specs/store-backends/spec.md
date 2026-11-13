## MODIFIED Requirements

### Requirement: A closed store answers unavailable

After `close`, store operations SHALL fail as unavailable on every backend, except change subscriptions, which
SHALL end normally with no items. Existing change streams SHALL also end, discarding backlog. Nonstreaming HTTP
requests that need the store SHALL get `503 store-unavailable`. Worker stop SHALL NOT close the store.

#### Scenario: A request after the store is closed

- **WHEN** a host closes its store and a client then sends a message over HTTP to a system on that store
- **THEN** the answer is `503 store-unavailable`

#### Scenario: Health after the store is closed

- **WHEN** a host closes its store and a client then gets `/health`
- **THEN** the answer is `503`

#### Scenario: Every backend

- **WHEN** a store of each backend is closed and then asked for its time
- **THEN** each fails as unavailable

#### Scenario: A change subscription after close

- **WHEN** a system calls changes after its Store was closed, or a previous stream is polled after close
- **THEN** the stream ends with None instead of yielding a store error or Resync

