## ADDED Requirements

### Requirement: The menu follows the mockup's boards

The menu SHALL show each state as its board in the change's `mockup/` shows it, with the departures its design lists:
rich rows with an icon, a second line and text on the right; the context's host under its name; "Running N of M"; a
not-connected menu with the last error and "Connect to <context>"; a disconnecting menu with the grace period's bar, the
finishing turns and Stop Now; and the submenus' rows as drawn.

#### Scenario: Not connected after a failure

- **WHEN** the connection to `staging` failed
- **THEN** the header says "Not connected", a row says "Couldn't reach staging" with the error and its time, and the
  menu has no Running section

#### Scenario: A context's host

- **WHEN** the context `prod` has the server `https://user:secret@actors.internal/api`
- **THEN** its row shows `prod` over `actors.internal`, with no user, password or path

#### Scenario: Disconnecting

- **WHEN** the tray stops a worker with a 30 second grace period and 12 seconds have passed
- **THEN** the menu shows the bar at 40%, "18 s left", the finishing turns and Stop Now, and no Context section
