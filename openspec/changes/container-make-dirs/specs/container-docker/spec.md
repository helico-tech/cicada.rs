## ADDED Requirements

### Requirement: Folders reach Docker without touching what exists

`make_dirs` SHALL learn what exists from Docker's archive stat and SHALL upload only the folders it makes, owned by
uid and gid 0 with their modes, one upload per mount with a read-only root. A folder of the image SHALL keep its
owner and mode, and so SHALL an existing parent.

#### Scenario: A writable folder for user 1000

- **WHEN** `make_dirs` makes `/out` with mode 0o777 in a created container that runs as user 1000, which then writes
  `/out/a`
- **THEN** the program exits with code 0 and `/out/a` exists

#### Scenario: Existing folders keep their owner and mode

- **WHEN** `make_dirs` makes `/tmp/out` and `/run/postgresql` in a created `postgres:18` container
- **THEN** `/tmp` keeps mode 1777, `/run/postgresql` keeps its owner and mode, and `/tmp/out` exists

#### Scenario: A path below a file

- **WHEN** `make_dirs` makes `/etc/passwd/x`, where `/etc/passwd` is a file of the image
- **THEN** it fails with `Conflict` and makes nothing

#### Scenario: A read-only root

- **WHEN** a created container has `read_only_root` and a volume at `/work`, and `make_dirs` makes `/work/cache` and
  `/etc/x`
- **THEN** it fails with `InvalidSpec` and makes nothing; with `/work/cache` alone it makes the folder

### Requirement: Links on Docker are followed to a folder

A link at a named path or a parent SHALL be followed: to a folder it counts as that folder, else it is `Conflict`.
The walk of one named folder SHALL follow at most 8 links and SHALL fail with `Conflict` on one more. Docker's stat
resolves a chain of links itself, so a chain counts as one link, and a link loop SHALL fail the call with `Other`, the
kind of Docker's refusal.

#### Scenario: A folder through a linked parent

- **WHEN** `make_dirs` makes `/var/run/app` in a created `postgres:18` container, where `/var/run` links to `/run`
- **THEN** it returns `Ok`, `/run/app` is a folder, and `/var/run` is still the link
