## ADDED Requirements

### Requirement: Containers without their actor are reported

`OrphanWatch::run` SHALL, at each interval until stopped, list the containers with the job's prefix label, and SHALL
call `on_orphan` once for each container whose container actor was not found in two rounds in a row, without removing
it. A failed list or actor read SHALL be retried in the next round and SHALL not count as a round without the actor.

#### Scenario: A container left without its actor

- **WHEN** a labelled container runs and no actor exists for its name, over three rounds
- **THEN** `on_orphan` is called once with its name, after the second round, and the container still runs

#### Scenario: An actor that appears

- **WHEN** an orphan was reported and its actor is then created
- **THEN** it is dropped from the set and not reported again while its actor exists

#### Scenario: A removed orphan

- **WHEN** a reported orphan is removed from the runtime
- **THEN** it leaves the set, and a new container of that name starts unreported

#### Scenario: A failed list

- **WHEN** `list` fails with `DaemonUnavailable` in one round
- **THEN** the watch goes on, and that round does not count toward an orphan
