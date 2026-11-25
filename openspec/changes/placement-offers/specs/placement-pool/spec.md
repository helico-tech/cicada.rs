## ADDED Requirements

### Requirement: Nodes offer places to a placement pool

`cicada-actors` SHALL provide the ready-made actor `cicada.placement/<pool>`, to which a node offers its labels and
number of places with `Offer`; each offer SHALL replace the node's last one and record the store time of its turn as
when the node was heard.

#### Scenario: A renewed offer

- **WHEN** a node offers `arm64` with 4 places and later `arm64, docker` with 2
- **THEN** the pool holds that node with `arm64, docker` and 2 places, heard at the second offer's time

### Requirement: A node that goes quiet is silent, once

The pool SHALL mark a node not heard from for its silence time as silent, send one notice of kind `silent` to its alert
target for that silence, and mark it live again with a notice of kind `back` at its next offer.

#### Scenario: Silence and return

- **WHEN** a node's last offer is older than the silence time over three checks, and then it offers again
- **THEN** one `silent` notice goes out, then one `back` notice, and the node is live

#### Scenario: Forgetting a node

- **WHEN** an operator sends `Forget` for a silent node, and for a live one
- **THEN** the silent node is removed, and the live one is refused with a reason
