## ADDED Requirements

### Requirement: Work asks for a place and gets a node

`Ask` SHALL grant a place on a live node that has every need and a free place, choosing the most free places and then
the smaller node id, with a grant number that only goes up, and SHALL send `granted` with the grant and the node to the
asker's target. `Release` SHALL free the place only when holder and grant match.

#### Scenario: Most free places first

- **WHEN** two live nodes have `arm64`, one with 1 free place and one with 3, and work asks for `arm64`
- **THEN** it is granted on the node with 3 free places

#### Scenario: A late release

- **WHEN** a holder releases with an older grant number than its current one
- **THEN** no place is freed

### Requirement: An ask is refused, queued or granted

An ask no node in the pool could meet SHALL be refused at once with the reason `noNode`; one that only silent or full
nodes could meet SHALL be queued, up to the pool's line limit (else refused with `lineFull`), and granted when a place
fits, skipping waiting asks that cannot be placed. A grant from the line whose notice never reached its holder SHALL be
freed. An ask to a target that is gone SHALL record nothing, and an out-of-bounds message SHALL be refused for good.

#### Scenario: Waiting for a node to come back

- **WHEN** the only node with `gpu` is silent and work asks for `gpu`, and the node then offers again
- **THEN** the ask is queued, and granted on that node after its offer

#### Scenario: A rare label does not block

- **WHEN** an ask for `gpu` waits and an ask for `arm64` comes after it while an `arm64` place is free
- **THEN** the `arm64` ask is granted at once

#### Scenario: No node has the needs

- **WHEN** no node in the pool has `gpu` and work asks for `gpu`
- **THEN** it gets `refused` with the reason `noNode` and nothing is recorded

#### Scenario: A full line

- **WHEN** the line holds `maxWaiting` asks and one more must wait
- **THEN** it gets `refused` with the reason `lineFull`

#### Scenario: A grant nobody received

- **WHEN** a waiting ask is granted from the line and its target is gone by then
- **THEN** the place is freed again

#### Scenario: An ask to a gone target

- **WHEN** work asks with a target that does not exist
- **THEN** the ask fails and the pool records nothing
