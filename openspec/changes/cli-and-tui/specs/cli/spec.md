# Spec Delta

## Purpose

The `cicada` command line and terminal UI: how it finds its server, what commands it has, and how it shows results.

## ADDED Requirements

### Requirement: The CLI finds its server from flags, environment and a config file

The server URL SHALL be `--server`, else `CICADA_SERVER`, else the `server` of the chosen context. The context SHALL
be `--context`, else `CICADA_CONTEXT`, else the config file's `current-context`. A named context that the file does
not have SHALL be an error.

#### Scenario: A flag wins over the environment and the file

- **WHEN** the file's current context has server `http://a`, `CICADA_SERVER` is `http://b` and `--server http://c` is
  given
- **THEN** the command talks to `http://c`

#### Scenario: An unknown context

- **WHEN** `--context nope` is given and the file has no context `nope`
- **THEN** the command fails with exit code 1 and a message naming `nope`, and sends no request

#### Scenario: No server anywhere

- **WHEN** no flag, no environment variable and no config file gives a server
- **THEN** the command fails with exit code 1 and says how to set one

### Requirement: The CLI sends the configured credentials

The bearer token SHALL be `--token`, else `CICADA_TOKEN`, else the context's `token`, and SHALL be sent as
`Authorization: Bearer <token>`. The context's `headers` SHALL be sent with every request.

#### Scenario: A token from the context

- **WHEN** the current context has `token = "t1"` and no `--token` or `CICADA_TOKEN` is given
- **THEN** every request carries `Authorization: Bearer t1`

### Requirement: The CLI reads its config file from a fixed path

The config file SHALL be `$CICADA_CONFIG`, else `$XDG_CONFIG_HOME/cicada/config.toml`, else
`$HOME/.config/cicada/config.toml`. A missing file SHALL count as an empty one. On Unix, a command that writes the
file SHALL leave it with mode 0600.

#### Scenario: XDG config home

- **WHEN** `CICADA_CONFIG` is unset and `XDG_CONFIG_HOME` is `/x`
- **THEN** the file read is `/x/cicada/config.toml`

#### Scenario: The file stays private

- **WHEN** on Unix `cicada config set-context` writes the file, which existed with mode 0644
- **THEN** the file has mode 0600 afterwards

### Requirement: The CLI has kubectl-style commands for every API operation

The binary SHALL offer, verb first, a command for each operation of the actor API (`get`, `describe`, `create`,
`delete`, `send`, `cancel`, `retry`, `deliver-now`, `pause`, `resume`, `purge`, `stats`, `health`) and `config`
commands that list, show, choose and set contexts. A missing item SHALL fail with exit code 1 and a message naming it.
An error response SHALL fail with exit code 1 and print the problem's title and detail.

#### Scenario: Send, then read the message

- **WHEN** `cicada send account/acme:7 --type Deposit --payload '{"amount":5}'` succeeds and the message is handled
- **THEN** it prints the message id, and `cicada get message <id>` shows status `done`

#### Scenario: A missing actor

- **WHEN** `cicada get actor account/nobody:1` is run and the actor does not exist
- **THEN** it fails with exit code 1 and a message naming `account/nobody:1`

### Requirement: The CLI prints tables, JSON or YAML

`get` SHALL print a table with a header row by default, more columns with `-o wide`, the API's JSON view with
`-o json` and the same as YAML with `-o yaml`. `describe` SHALL print YAML unless `-o json` is given. A listing SHALL
print one page of `--limit` items (default 100); `--all` SHALL follow the cursor to the last page.

#### Scenario: JSON is the API view

- **WHEN** `cicada get actor account/acme:7 -o json` is run
- **THEN** it prints the JSON that `GET /actors/account/acme:7` returns, pretty-printed

### Requirement: The TUI browses the system

`cicada tui` SHALL show actors, pending messages, outcomes, running turns, nodes and types, refresh the shown screen
every 2 s, filter its rows as the user types, sort them by a column, and drill down from a row to an overview of the
actor, message, node or type, from which Esc goes back.

#### Scenario: Switch view

- **WHEN** the user types `:outcomes failed` and Enter
- **THEN** the TUI shows the failed outcomes and keeps refreshing them

#### Scenario: Drill down and back

- **WHEN** the user presses Enter on a running turn, then Esc
- **THEN** the TUI shows the overview of the turn's actor, then the turns again

### Requirement: The TUI shows the state of the system at a glance

The TUI SHALL show in its header the context, the server, the nodes by health, the number of actors, pending messages
and running turns, and how long the oldest due message has waited, and SHALL colour status values by meaning.

#### Scenario: A failed outcome stands out

- **WHEN** the outcomes include a failed message
- **THEN** its row is drawn in red

### Requirement: The TUI acts only after confirmation

The TUI SHALL run delete, cancel, retry, pause/resume, purge and deliver-now on the selected item only after the user
confirms.

#### Scenario: Confirm before acting

- **WHEN** the user selects a pending message, presses `c` and then `y`
- **THEN** the message is cancelled; pressing `n` or `Esc` instead cancels nothing

### Requirement: The TUI never waits on HTTP

HTTP requests SHALL run outside the drawing loop, so the screen keeps answering keys while a request is slow. A result
for a view the user has left SHALL be dropped. The terminal SHALL be restored on exit, on error and on panic.

#### Scenario: A slow node

- **WHEN** the node takes longer than a refresh to answer
- **THEN** the TUI still switches views and moves the selection, and shows the rows once they arrive
