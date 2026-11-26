## ADDED Requirements

### Requirement: The menu shows the worker

The tray SHALL turn its state, its turns, the registered types, the worker's types and the contexts into one menu model
that a plain function computes on every platform, and on macOS SHALL show it as a status item and a native menu that
stays current while it is open.

#### Scenario: A running turn in the menu

- **WHEN** the worker is online and a turn has run for 3 seconds
- **THEN** the menu model's Running section lists its actor, message type and attempt with 3 seconds since the claim

#### Scenario: The recent turns badge

- **WHEN** the last finished turns hold two failed or abandoned turns
- **THEN** the Recent Turns submenu's badge is 2

#### Scenario: The types the worker runs

- **WHEN** the worker started with two of three registered types
- **THEN** the Registered Types submenu marks those two

### Requirement: Quit waits for the worker

Quitting SHALL disconnect the worker and end the app only once the tray is `Offline`, without blocking the main thread,
whether it comes from the Quit item, Ctrl-C or a system terminate; on macOS before 14.0 `run` SHALL refuse with an
error.

#### Scenario: Quit while a turn runs

- **WHEN** a turn is running and the user picks Quit
- **THEN** the tray goes `Stopping`, the menu stays responsive, and the app ends once the tray is `Offline`

#### Scenario: A system terminate

- **WHEN** the system asks the app to terminate while the worker is online
- **THEN** the app answers that it terminates later, and confirms once the tray is `Offline`
