## MODIFIED Requirements

### Requirement: Files are copied only before start

`copy_in` SHALL write the given files into a `Created` container, and SHALL fail with `Conflict` for a container
that is missing, running or ended. Every runtime SHALL fail with `InvalidSpec`, before any write, for a path that is
relative, has a `..` part, names no file once empty and `.` parts are dropped, equals the target of a volume or
bind mount of the container, or lies at or below a tmpfs target with no deeper volume or bind mount.

#### Scenario: Copying into a running container

- **WHEN** `copy_in` is called for a running container
- **THEN** it fails with `Conflict` and no file is written

#### Scenario: Copying onto a mount target

- **WHEN** a created container has a volume at `/work`, and `copy_in` writes `/work` and `/etc/b`
- **THEN** it fails with `InvalidSpec` and writes nothing, on the fake and on Docker, with a read-only or a writable
  root

#### Scenario: Copying onto a tmpfs of a writable root

- **WHEN** a created container has a writable root and a tmpfs at `/scratch`, and `copy_in` writes `/scratch/c`
- **THEN** it fails with `InvalidSpec` and writes nothing, on the fake and on Docker

#### Scenario: One file under two spellings

- **WHEN** `copy_in` writes `/work/./a` and then `/work/a` with other contents
- **THEN** the container holds one file `/work/a`, with the second contents

#### Scenario: Copying to the root path

- **WHEN** `copy_in` writes a file at `/`
- **THEN** it fails with `InvalidSpec` and writes nothing

### Requirement: Networks, volumes and sweeps are safe to repeat

`ensure_network` and `ensure_volume` SHALL find an existing network or volume by name and return `Ok`. `sweep` SHALL
remove every container, then every network, then every volume that has the given label, and SHALL return `Ok` when
nothing has it. A network or volume that is still in use SHALL be skipped with its reason while the sweep goes on;
only an unavailable daemon SHALL end a sweep with an error. Its result SHALL name nothing already gone or being
removed by another caller.

#### Scenario: A repeated ensure keeps one network

- **WHEN** `ensure_network` is called twice with the same internal network spec
- **THEN** both calls return `Ok` and one network with that name exists

#### Scenario: A sweep removes by label

- **WHEN** containers, networks and volumes carry the label `run=r1` and others carry `run=r2`, and `sweep` is called
  with `run=r1`
- **THEN** the ones labelled `run=r1` are gone, the others remain, and the result names what was removed

#### Scenario: A sweep skips a network in use

- **WHEN** a network labelled `run=r1` is used by a container without that label, and `sweep` is called with `run=r1`
- **THEN** it returns `Ok`, the network remains, and the result names it as skipped with its reason

#### Scenario: Something is gone before the sweep removes it

- **WHEN** a container, a network or a volume labelled `run=r1` is listed by `sweep` and removed by another caller
  before the sweep removes it, or a container is still being removed by another caller (Docker's 409)
- **THEN** the sweep goes on and returns `Ok`, and that name is neither in its kind's removed list nor in `skipped`
