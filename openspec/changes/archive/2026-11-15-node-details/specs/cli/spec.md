## ADDED Requirements

### Requirement: Node displays show optional machine details

Standard node tables SHALL add HOSTNAME after ID. Wide output SHALL retain HOSTED and add VERSION and LABEL.
Absent metadata SHALL use the existing `-` marker. The TUI SHALL use the shared node columns, including wide details.
Node JSON/YAML output SHALL follow the API view, with absent metadata omitted.

#### Scenario: Node list and detail output

- **WHEN** get nodes is run in standard or wide mode
- **THEN** its columns show the specified details and missing-value markers
- **WHEN** a node is described or printed as JSON/YAML
- **THEN** hostname, cicadaVersion and label follow the optional API fields

#### Scenario: TUI node view

- **WHEN** the nodes screen loads a row with details or one from an old server
- **THEN** its standard grid shows the hostname or `-`, and wide mode exposes version and label
- **AND** the existing selection, filtering and node detail navigation remain available

#### Scenario: Control characters in display metadata

- **WHEN** a hostname or label contains a newline, carriage return, tab or another Unicode control character
- **THEN** table and TUI metadata cells show `\n`, `\r`, `\t` or `\u{hex}` respectively, preserving printable Unicode
- **AND** the stored value and its JSON/YAML value remain unchanged
