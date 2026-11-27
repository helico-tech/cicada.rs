# menu-bar-tray Specification

## Purpose
TBD - created by archiving change tray-menu-bar. Update Purpose after archive.

## Requirements

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

Quitting SHALL disconnect the worker and end the app only once the tray holds no connection (`Offline` or `Failed`),
without blocking the main thread, whether it comes from the Quit item, Ctrl-C or a system terminate. On macOS before
14.0 `run` SHALL refuse with an error; before 14.4 the running turns SHALL show their details in the title.

#### Scenario: Quit while a turn runs

- **WHEN** a turn is running and the user picks Quit
- **THEN** the tray goes `Stopping`, the menu stays responsive, and the app ends once the tray is `Offline`

#### Scenario: A system terminate

- **WHEN** the system asks the app to terminate while the worker is online
- **THEN** the app answers that it terminates later, and confirms once the tray is `Offline`

#### Scenario: A stop that fails

- **WHEN** the user quits and the worker's stop ends with an error, so the tray is `Failed`
- **THEN** the app ends as for `Offline`

#### Scenario: An old macOS

- **WHEN** `run` starts on macOS 13
- **THEN** it returns exit code 1 with an error that the menu needs macOS 14

#### Scenario: A running turn before macOS 14.4

- **WHEN** a turn runs and the app runs on macOS 14.2
- **THEN** the turn's item shows the actor, message type and attempt in its title, with no subtitle

#### Scenario: A context pick after a failure

- **WHEN** the tray is `Failed` and the user picks a context
- **THEN** the tray connects to it at once

### Requirement: Stop Now in the menu

While the tray stops a worker that was ready, the menu SHALL offer Stop Now, and `Tray::stop_now` SHALL end the
worker's grace period: its running turns SHALL be cancelled, and the state SHALL become `Offline` once the worker
returns, within the cancel timeout. In any other state `stop_now` SHALL do nothing and the menu SHALL not offer it.

#### Scenario: Stop Now during the grace period

- **WHEN** a turn runs, the tray disconnects with a grace period of 30 seconds, and the user picks Stop Now
- **THEN** the turn is cancelled and the tray is `Offline` within the cancel timeout

#### Scenario: Stop Now while online

- **WHEN** the tray is online and `stop_now` is called
- **THEN** nothing changes and the menu offers no Stop Now

### Requirement: Turns after Stop Now

After Stop Now, a turn that ends without an acknowledged commit SHALL be listed as abandoned, whether its lease was
released or left to expire, and a turn whose handler had ended MAY still commit. A turn the worker leaves behind SHALL
stay listed as running until it ends; `Offline` SHALL NOT wait for it.

#### Scenario: A cancelled turn gives its lease back

- **WHEN** the user picks Stop Now while a turn's handler runs
- **THEN** the turn is listed as abandoned once its lease is given back, and its message is pending again

#### Scenario: Stop Now while the release is slow

- **WHEN** the user picks Stop Now and the cancelled turn's release is still in flight at the cancel timeout
- **THEN** the tray is `Offline` at the cancel timeout, the turn stays listed as running, and it is listed as
  abandoned once the release ends
