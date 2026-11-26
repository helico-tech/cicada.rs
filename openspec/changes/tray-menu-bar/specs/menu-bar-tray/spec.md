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

### Requirement: Stop Now

While the worker is stopping, `Tray::stop_now` SHALL end it at once: the running turns SHALL be listed as abandoned and
the state SHALL become `Offline`. In any other state it SHALL do nothing.

#### Scenario: Stop Now during the grace period

- **WHEN** a turn is running, the tray disconnects, and `stop_now` is called before the grace period ends
- **THEN** the state is `Offline` and the turn is listed as abandoned
