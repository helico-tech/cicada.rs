## ADDED Requirements

### Requirement: A full pool says so once

When a pool knows `maxNodes` nodes, it SHALL accept and ignore an offer from a node it does not know, set a pool-level
`full` mark with that node and the offer's time, and send one `full` notice when the mark is set. Later ignored offers
SHALL move the mark without a notice. The mark SHALL end one silence time after the last ignored offer, or when an offer
adds a new node. An offer of a key another live node holds SHALL be handled as a contest before the full check.

#### Scenario: A new node offers to a full pool

- **WHEN** a pool with `maxNodes` 1 knows one node and a second node offers three times
- **THEN** every offer's turn commits, the second node is not added, and one `full` notice goes out

#### Scenario: The full period ends

- **WHEN** no new node offers for one silence time after the last ignored offer, and then one does
- **THEN** the mark was removed and a second `full` notice goes out

#### Scenario: The pool fills again

- **WHEN** a full pool's `maxNodes` is raised, a new node is added, and the pool is full again before one silence time
  passed, and another new node offers
- **THEN** a second `full` notice goes out
