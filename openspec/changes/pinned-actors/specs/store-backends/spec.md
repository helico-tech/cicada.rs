## MODIFIED Requirements

### Requirement: A host reads the store's time and the next due time through the system

`ActorSystem` SHALL answer the store's time with `now`. With `next_due_at` it SHALL answer the earliest store time
at which an instance of a type the system hosts may become claimable: the delivery time of the head of its mailbox,
or the end of its lease when that is later. Paused instances and actors pinned to a different NodeConfig.node_id SHALL NOT count. The answer SHALL be nothing
when no eligible message is pending. The internal dispatcher SHALL use the same owner filter for its sleep calculation.

#### Scenario: The store's time

- **WHEN** a test on virtual time moves the clock forward by one hour and asks the system for `now`
- **THEN** the answer is one hour later than before

#### Scenario: A message that is due later

- **WHEN** a message for a hosted type is sent with a delay of ten minutes and nothing else is pending
- **THEN** `next_due_at` answers the time ten minutes from now

#### Scenario: Nothing pending

- **WHEN** no message is pending for a hosted type
- **THEN** `next_due_at` answers nothing

#### Scenario: A paused actor

- **WHEN** the only pending message belongs to a paused actor
- **THEN** `next_due_at` answers nothing


#### Scenario: Only another owner has due work

- **WHEN** the only due messages for hosted types belong to actors pinned to another owner
- **THEN** next_due_at answers nothing and those actors do not force immediate dispatcher wakeups
- **AND** unpinned and matching-owner messages retain the existing due/lease calculation
