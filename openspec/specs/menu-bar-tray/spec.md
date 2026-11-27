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
- **THEN** the context becomes the selected one, the tray does not connect, and Connect or the switch connects to it

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

### Requirement: The tray's settings

The tray SHALL keep, per context, the types its worker runs and its concurrency in `tray.toml` next to the config file,
and SHALL pass them to the worker as `--types` and `--concurrency` whenever the context has saved settings. Settings
SHALL change the context's server and token through the same locked code as `cicada config`, keeping other contexts
as they are on disk.

#### Scenario: Fewer types

- **WHEN** the settings of context `prod` pick one of two registered types and the tray connects to `prod`
- **THEN** the worker starts with `--types` naming that one type

#### Scenario: Another writer meanwhile

- **WHEN** the Settings window is open, `cicada config set-context staging` adds a context, and the user applies a
  change to `prod`
- **THEN** the config file holds both `staging` and the changed `prod`

#### Scenario: Apply while connected

- **WHEN** the tray is online with `prod` and the user applies settings for `prod`
- **THEN** the tray disconnects, waits for no connection, and connects again with the new arguments

#### Scenario: An invalid server

- **WHEN** the server field holds `ftp://x` and the user applies
- **THEN** nothing is written and the window shows the error under the server field

#### Scenario: All types over the environment

- **WHEN** `CICADA_TYPES` names one type and the settings of `prod` pick all types
- **THEN** the worker starts with `--types` naming all types

#### Scenario: No registered type left

- **WHEN** the saved settings of `prod` name only types the binary no longer registers, and the tray connects to `prod`
- **THEN** the tray is `Failed` with an error that tells the user to open Settings, and no worker starts

#### Scenario: A type literally named all or none

- **WHEN** the saved settings of `prod` pick only the registered type named `all`, or only the one named `none`, and
  another type is registered
- **THEN** the tray is `Failed` with an error that tells the user to open Settings, and no worker starts

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
