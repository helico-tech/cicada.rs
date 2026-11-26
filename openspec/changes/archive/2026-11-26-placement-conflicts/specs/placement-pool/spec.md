## MODIFIED Requirements

### Requirement: Nodes offer places to a placement pool

`cicada-actors` SHALL provide the ready-made actor `cicada.placement/<pool>`, created by the host with its silence time
and node limit in its state, to which a node offers its instance, labels and number of places with `Offer`; each offer
SHALL replace the node's last one and record the store time of its turn as when the node was heard, except an offer with
another instance than the recorded one while the node is live, which SHALL change no entry field, SHALL mark the node in
conflict, and SHALL send one `conflict` alert while the mark lasts. The mark SHALL end when the node goes silent, or
when the recorded instance offers after no conflicting offer for one silence time.

#### Scenario: A renewed offer

- **WHEN** a node offers `arm64` with 4 places and later `arm64, docker` with 2
- **THEN** the pool holds that node with `arm64, docker` and 2 places, heard at the second offer's time

#### Scenario: Two machines under one id

- **WHEN** a second machine offers under a live node's id with another instance, three times
- **THEN** the node keeps the first machine's labels and places, and one `conflict` alert goes out

#### Scenario: A replaced machine

- **WHEN** the first machine stops offering, the node goes silent, and the second machine offers
- **THEN** the second machine's offer takes over the entry and the conflict mark is cleared

#### Scenario: The second machine stops

- **WHEN** the second machine stops offering for one silence time and the first keeps offering, and later the second
  offers again
- **THEN** the mark is cleared after that silence time, and the later offer marks the node again with one new alert

#### Scenario: A third instance

- **WHEN** a node is marked in conflict and a third instance offers under its id
- **THEN** the mark holds the third instance and no new alert goes out
