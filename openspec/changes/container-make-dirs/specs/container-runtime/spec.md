## ADDED Requirements

### Requirement: Folders are made only where missing, before start

`make_dirs(name, dirs)` SHALL make each missing named folder with its mode and each missing parent with 0755 in a
`Created` container, and SHALL leave every existing folder as it is, mode and owner included. It SHALL fail before any
write with `Conflict` for a file, or a link to no folder, at a named path or a parent, or a container that is not
`Created`, and with `InvalidSpec` for a mode above 0o7777, one path with two modes, or a path `copy_in` refuses. Its
default SHALL fail with `Other`.

#### Scenario: A folder for a program that does not run as root

- **WHEN** `make_dirs` makes `/out` with mode 0o777 in a created container, twice
- **THEN** both calls return `Ok` and `/out` has mode 0o777

#### Scenario: An existing folder is left alone

- **WHEN** `/data` exists with mode 0o700, and `make_dirs` makes `/data` with mode 0o777 and `/data/new`
- **THEN** it returns `Ok`, `/data` keeps mode 0o700, and `/data/new` has mode 0o777

#### Scenario: A file where a folder was asked

- **WHEN** `/out` is a file, and `make_dirs` makes `/out` or `/out/sub`
- **THEN** it fails with `Conflict`, the file is unchanged and nothing is made

#### Scenario: A named parent keeps its own mode

- **WHEN** `make_dirs` makes `/a/b` and then `/a` with mode 0o777 in one call, where neither exists
- **THEN** `/a` has mode 0o777 and `/a/b` mode 0o755, and the same with the two in the other order

#### Scenario: One path named twice

- **WHEN** `make_dirs` names `/out` twice with mode 0o777, and in another call twice with modes 0o777 and 0o755
- **THEN** the first makes `/out` once with mode 0o777, and the second fails with `InvalidSpec` and makes nothing

#### Scenario: Bad modes and late calls

- **WHEN** `make_dirs` is called with mode 0o10000, and for a running container
- **THEN** the first fails with `InvalidSpec`, the second with `Conflict`, and nothing is made

#### Scenario: A runtime written before make_dirs

- **WHEN** a host's own runtime implements every method but `make_dirs`, and `make_dirs` is called on it
- **THEN** it compiles, and the call fails with `Other`
