## ADDED Requirements

### Requirement: The host chooses the node config after the types are known

`App::node_config_for` SHALL take a function that `serve` and `worker` call once, after every check of the flags, the
variables and the config file and before the wait for the store or the server, with the names of the types the run
hosts; its `NodeConfig` SHALL take the
place of the host's config, and the flags and their variables SHALL still override its fields. An error from the
function SHALL fail the command with exit code 1. The operator commands SHALL NOT call it.

#### Scenario: A node without the runner type

- **WHEN** the host's function returns the runner identity's node id only when the types hold the runner type, and the
  node starts with `--types` that leave it out
- **THEN** the node runs under the default node id and the function saw only the picked types

#### Scenario: A flag still wins

- **WHEN** the function returns a node id and the node starts with `--node-id other`
- **THEN** the node runs as `other`

#### Scenario: The function fails

- **WHEN** the function returns an error
- **THEN** the command exits with code 1 and prints the error, and the store is not opened

#### Scenario: A bad flag

- **WHEN** the node starts with a flag that cannot be used
- **THEN** the command exits with code 2 and the function is not called

#### Scenario: An operator command

- **WHEN** the binary runs `get actors`
- **THEN** the function is not called
