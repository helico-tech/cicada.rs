## MODIFIED Requirements

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
- **THEN** the context becomes the selected one, the tray does not connect, and Connect or the switch connects to it

## ADDED Requirements

### Requirement: The menu follows the mockup's boards

The menu SHALL show each state as its board in the change's `mockup/` shows it, with the departures its design lists:
rich rows with an icon, a second line and text on the right; the context's host under its name; "Running N of M"; a
not-connected menu with the last error and "Connect to <context>"; a disconnecting menu with the grace period's bar, the
finishing turns and Stop Now; and the submenus' rows as drawn.

#### Scenario: Not connected after a failure

- **WHEN** the connection to `staging` failed
- **THEN** the header says "Not connected", a row says "Couldn't reach staging" with a fixed label (Connection
  refused, No answer or Connection failed) and its time, no error text, and the menu has no Running section

#### Scenario: A context's host

- **WHEN** the context `prod` has the server `https://user:secret@actors.internal/api`
- **THEN** its row shows `prod` over `actors.internal`, with no user, password or path

#### Scenario: Disconnecting

- **WHEN** the tray stops a worker with a 30 second grace period and a 10 second cancel timeout, and 12 seconds have
  passed
- **THEN** the menu shows the bar at 30%, "28 s left", the finishing turns and Stop Now, and no Context section

#### Scenario: A turn left behind while not connected

- **WHEN** Stop Now made the tray `Offline` while a cancelled turn's release is still in flight
- **THEN** the menu lists the turn under "Finishing" until it ends
