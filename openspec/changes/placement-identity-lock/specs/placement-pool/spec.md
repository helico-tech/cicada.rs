## ADDED Requirements

### Requirement: A member notices another machine under its node id

Each round, `PlacementMember` SHALL read its own node row and, when its host name is set and differs from the machine's,
SHALL send no offer that round and send the pool `Duplicate { node, host }`; the pool SHALL mark the node in conflict
with that host, send one `conflict` alert per mark, give the node no new places while marked, and clear the mark one
silence time after the last report.

#### Scenario: A copied identity folder

- **WHEN** two machines run with the same identity and the node row's host name flips between them
- **THEN** each member that sees the other host skips its offers and reports it, and the pool alerts once

#### Scenario: A redeploy

- **WHEN** a node is replaced by a new container with another host name and the old one has stopped
- **THEN** no duplicate is reported

### Requirement: One process per identity folder

`NodeIdentity::load_or_create` SHALL take an exclusive lock on the identity folder before it reads or creates the file
and keep it while the identity or a clone of it lives; a second process SHALL fail with the kind `Locked`.

#### Scenario: A second process

- **WHEN** a node's member holds its identity and a second process loads the same folder
- **THEN** the second load fails with the kind `Locked`
