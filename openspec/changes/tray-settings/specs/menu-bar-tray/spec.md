## ADDED Requirements

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
