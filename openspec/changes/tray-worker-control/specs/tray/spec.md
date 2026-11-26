## ADDED Requirements

### Requirement: A tray connects and disconnects a worker

`cicada-tray` SHALL provide `Tray`, built from a function that builds an `App`; `connect(context)` SHALL run a worker
over HTTP for that context in the process with a fresh `App`, and `disconnect()` SHALL stop it in good order within
its grace period.

#### Scenario: Connect and disconnect

- **WHEN** a tray connects to a running server's context and then disconnects while a turn runs
- **THEN** its state goes `Connecting`, `Online`, `Stopping` and `Offline`, and the running turn finishes

#### Scenario: A server that does not answer

- **WHEN** a tray connects to a context whose server is unreachable
- **THEN** its state ends `Failed` with the error

### Requirement: A tray shows the turns that run

The tray SHALL record each running turn (actor, message type, attempt, claim time) and keep the last 50 finished turns
with their outcome.

#### Scenario: A turn that runs

- **WHEN** the connected worker handles a message
- **THEN** the turn is listed as running, and then among the finished turns with its outcome

### Requirement: An App lists its registered types

`App::type_names()` SHALL return the names of the hosted types in the order of the `host` calls.

#### Scenario: Two hosted types

- **WHEN** an `App` hosts `orders` and then `invoices`
- **THEN** `type_names()` returns `["orders", "invoices"]`
