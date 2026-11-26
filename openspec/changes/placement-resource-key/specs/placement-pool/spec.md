## ADDED Requirements

### Requirement: One live node holds a resource

An offer MAY name a resource key. The pool SHALL ignore an offer whose key a different live node holds, record the
contest of that key and node in a pool-level list with the time of the last such offer, send one `conflict` alert when
the contest is recorded, and remove it one silence time after its last offer; the next offer with the key SHALL take it
once its holder is silent or gone. A stored state with two live nodes holding one key SHALL be refused.

#### Scenario: A new node offering a held daemon

- **WHEN** a node that the pool does not know offers a daemon id another live node holds, three times
- **THEN** the node is not added and one `conflict` alert goes out

#### Scenario: A known node offering a held daemon

- **WHEN** a known node starts offering a daemon id another live node holds, goes silent, and offers it again within one
  silence time
- **THEN** its offers change nothing and only one `conflict` alert goes out

#### Scenario: The holder stops

- **WHEN** the first node goes silent and the second keeps offering the key
- **THEN** the second node's offer takes the key and its places count
