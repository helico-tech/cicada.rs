## ADDED Requirements

### Requirement: Nodes offer places to a placement pool

`cicada-actors` SHALL provide the ready-made actor `cicada.placement/<pool>`, created by the host with its silence time
and node limit in its state, to which a node offers its instance, labels and number of places with `Offer`; each offer
SHALL replace the node's last one and record the store time of its turn as when the node was heard.

#### Scenario: A renewed offer

- **WHEN** a node offers `arm64` with 4 places and later `arm64, docker` with 2
- **THEN** the pool holds that node with `arm64, docker` and 2 places, heard at the second offer's time

### Requirement: A node that goes quiet is silent, once

The pool SHALL mark a node not heard from for its silence time as silent, send one notice of kind `silent` to its alert
target for that silence, and mark it live again with a notice of kind `back` at its next offer.

#### Scenario: Silence and return

- **WHEN** a node's last offer is older than the silence time over three checks, and then it offers again
- **THEN** one `silent` notice goes out, then one `back` notice, and the node is live

#### Scenario: A lost check

- **WHEN** the pool's armed `Check` is purged and someone sends `Rearm`
- **THEN** a fresh check is armed at the old time and silent nodes are still found

#### Scenario: A failing alert target

- **WHEN** the alert target refuses the pool's notice
- **THEN** only that `Alert` turn fails, and the node is marked silent all the same
