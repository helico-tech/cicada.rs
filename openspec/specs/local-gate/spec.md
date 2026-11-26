# local-gate Specification

## Purpose
The local gate records check evidence for the exact committed tree (ADR 0046) and lets a commit that changes only
allowed files reuse the nearest full pass (ADR 0070).

## Requirements

### Requirement: Changes under openspec reuse the last full pass

The gate's docs shortcut SHALL accept any non-executable regular file under `openspec/` as a change that reuses the
nearest full pass, and SHALL still refuse symlinks, mode changes and executable files there and non-Markdown files
elsewhere. Review markers SHALL be written as non-executable files.

#### Scenario: An archive commit

- **WHEN** a commit after a full pass only moves a change into `openspec/changes/archive`, with its `.openspec.yaml`
  and review markers
- **THEN** the gate reuses the full pass and runs only the docs checks

#### Scenario: A symlink under openspec

- **WHEN** a commit adds a symlink under `openspec/`
- **THEN** the shortcut is refused and a full run is needed
