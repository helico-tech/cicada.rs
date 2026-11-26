## ADDED Requirements

### Requirement: A member offers a node's places while it can run work

`PlacementMember::run` SHALL, every interval until stopped, call the host's probe and send an `Offer` with the node's
own id (`ActorSystem::node_id`) and its identity's instance when the probe gives a capacity, and send nothing when it
gives none; a failed send SHALL be tried in the next round, and `run` SHALL end on its stop also while a probe hangs.

#### Scenario: A runtime that stops answering

- **WHEN** the probe returns a capacity for three rounds and then nothing
- **THEN** the pool gets three offers, then none, and marks the node silent after its silence time

#### Scenario: A node id from the command line

- **WHEN** the node runs with `--node-id` set to another id than the identity file's
- **THEN** the member offers the node's own id, the one its pins use

#### Scenario: A zero interval

- **WHEN** a host builds `MemberSettings` with an interval of zero
- **THEN** it is refused

### Requirement: A node keeps its identity across restarts

`NodeIdentity::load_or_create` SHALL give the same node id and instance across restarts, SHALL create a missing folder
and file, and SHALL fail with `InvalidIdentity` for a file it cannot read or parse, never making a new identity then.

#### Scenario: A restart

- **WHEN** a node's process restarts and loads its identity file again
- **THEN** it gets the same node id and instance

#### Scenario: A corrupt identity file

- **WHEN** the identity file holds text that is not the identity JSON
- **THEN** `load_or_create` fails with `InvalidIdentity` and leaves the file as it is
