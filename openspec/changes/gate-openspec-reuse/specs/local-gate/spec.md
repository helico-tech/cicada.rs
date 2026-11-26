## ADDED Requirements

### Requirement: Changes under openspec reuse the last full pass

The gate's docs shortcut SHALL accept any regular file under `openspec/`, plain or executable, as a change that reuses
the nearest full pass, and SHALL still refuse symlinks there and non-Markdown or executable files elsewhere.

#### Scenario: An archive commit

- **WHEN** a commit after a full pass only moves a change into `openspec/changes/archive`, with its `.openspec.yaml`
  and executable review markers
- **THEN** the gate reuses the full pass and runs only the docs checks

#### Scenario: A symlink under openspec

- **WHEN** a commit adds a symlink under `openspec/`
- **THEN** the shortcut is refused and a full run is needed
