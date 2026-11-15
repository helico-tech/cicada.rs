## MODIFIED Requirements

### Requirement: The settings of the node come from flags, variables and the host

Node flags and their CICADA variables SHALL set NodeConfig for serve and direct-store workers: node-id,
node-label, concurrency, lease-ttl, heartbeat-interval, poll-interval, shutdown-grace-period, max-turn and
max-lost-claims. Precedence SHALL be flag, nonempty variable, host config, then library default. HTTP workers SHALL
map only applicable settings and refuse explicit settings they cannot honor as specified below.

#### Scenario: A variable over the host's value

- **WHEN** the host gave a `NodeConfig` with a concurrency of 4 and `CICADA_CONCURRENCY` is 8
- **THEN** the node runs at most 8 turns at once

#### Scenario: The host's value over the default

- **WHEN** the host gave a `NodeConfig` with a concurrency of 4 and neither the flag nor the variable is set
- **THEN** the node runs at most 4 turns at once

#### Scenario: The limit of lost claims is turned off

- **WHEN** the binary runs `serve --max-lost-claims off`
- **THEN** the node has no limit of lost claims

#### Scenario: A value that cannot be read

- **WHEN** a direct-store command reads `CICADA_LEASE_TTL` as `soon`
- **THEN** the command fails with exit code 2, and the error names `CICADA_LEASE_TTL`

#### Scenario: Label precedence

- **WHEN** a label is supplied by flag, variable and host config
- **THEN** --node-label wins, else CICADA_NODE_LABEL, else the host's label, else None
- **AND** an empty environment value is unset, while whitespace-only is invalid
- **AND** valid leading/trailing spaces are preserved from the variable as from the flag or host configuration
- **AND** invalid explicit values are usage errors (exit 2); invalid host config follows start validation (exit 1)

## ADDED Requirements

### Requirement: HTTP workers refuse explicit node labels

An App worker over HTTP SHALL refuse --node-label because it has no node row. It SHALL ignore CICADA_NODE_LABEL and
the host's NodeConfig.label without validating them. This SHALL NOT change WorkerConfig or remote lease identity.

#### Scenario: Label flag in HTTP mode

- **WHEN** worker --server or --context selects HTTP execution with --node-label
- **THEN** it fails with exit 2 and a diagnostic explaining that a worker over HTTP has no node row

#### Scenario: Unused label cannot break HTTP execution

- **WHEN** HTTP execution has a malformed CICADA_NODE_LABEL or an invalid host label but no --node-label flag
- **THEN** it does not read or validate either label, and applicable worker settings still apply normally
