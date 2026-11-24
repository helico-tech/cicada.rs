## MODIFIED Requirements

### Requirement: Files reach a read-only container only on a mount

With `read_only_root`, `copy_in` SHALL write a file only when its path lies below the target of a volume or bind
mount of the container, and SHALL fail with `InvalidSpec` before any write when a path lies elsewhere, a tmpfs
included.

#### Scenario: Copying onto a volume of a read-only container

- **WHEN** a created container has `read_only_root` and a volume at `/work`, and `copy_in` writes `/work/a` and `/etc/b`
- **THEN** it fails with `InvalidSpec` and writes nothing; with `/work/a` alone it writes the file, read back after start

## ADDED Requirements

### Requirement: A list leaves out what it cannot report

`list` SHALL leave out a container that was removed after Docker listed it, and one whose state Docker does not
report, and SHALL return the others. `inspect`, `start`, `copy_in` and `stop` of a container whose state Docker does
not report SHALL still fail with `Other`.

#### Scenario: A container without a state

- **WHEN** Docker lists two containers with the label and reports no state for one of them, and `list` is called
- **THEN** it returns `Ok` with the other container only
