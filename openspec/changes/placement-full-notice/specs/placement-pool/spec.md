## ADDED Requirements

### Requirement: A full pool says so once

When a pool knows `maxNodes` nodes, it SHALL accept and ignore an offer from a node it does not know, set a pool-level
`full` mark with that node and the offer's time, and send one `full` notice when the mark is set. Later ignored offers
SHALL move the mark without a notice. The mark SHALL end one silence time after the last ignored offer.

#### Scenario: A new node offers to a full pool

- **WHEN** a pool with `maxNodes` 1 knows one node and a second node offers three times
- **THEN** every offer's turn commits, the second node is not added, and one `full` notice goes out

#### Scenario: The full period ends

- **WHEN** no new node offers for one silence time after the last ignored offer, and then one does
- **THEN** the mark was removed and a second `full` notice goes out
