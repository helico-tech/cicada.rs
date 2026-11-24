## ADDED Requirements

### Requirement: Docker's archive is read as a stream

`copy_out` SHALL read Docker's archive of `path` as a stream and SHALL hold no more than the kept files' bytes: the
data of a skipped file SHALL be passed over, not stored. An error during the stream SHALL fail the whole call with
its kind, with no partial result. It SHALL work on a created, running or exited container, volumes included; files
written to a tmpfs SHALL be gone once the container ended.

#### Scenario: Artifacts after the end

- **WHEN** a container wrote files to its root file system and to a volume and exited, and `copy_out` is called for
  each folder before `remove`
- **THEN** both return their files

#### Scenario: The stream fails in the middle

- **WHEN** Docker's archive stream fails after the first file, with a lost connection
- **THEN** `copy_out` fails with `DaemonUnavailable` and returns no files

#### Scenario: The container is removed during the call

- **WHEN** the container is found, then removed before Docker sends its archive, which answers 404
- **THEN** `copy_out` inspects again, finds no container, and fails with `Conflict`

#### Scenario: A tmpfs after the end

- **WHEN** a container with a tmpfs at `/scratch` wrote `/scratch/a` and exited, and `copy_out("/scratch")` is called
- **THEN** the result holds no `/scratch/a`

#### Scenario: A progress file while running

- **WHEN** a running container keeps renaming a fresh `/work/progress.json` into place, and `copy_out` is called
  for that file
- **THEN** it returns one whole file
