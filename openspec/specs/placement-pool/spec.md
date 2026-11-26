# placement-pool Specification

## Purpose
Nodes offer places to a ready-made placement pool, and work asks it for a place on a node that has what it needs;
the answer names a node to pin a child to (ADR 0052, ADR 0067).

## Requirements

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

### Requirement: A deleted holder frees its place

The pool SHALL watch an ask's target with `watch_existing` from the commit that first records the ask, granted or
queued, and on a `Stopped` for that target SHALL free every place it holds and remove its waiting asks, exactly once,
and SHALL unwatch a target that holds and waits for nothing. An ask whose target is the pool itself SHALL be refused.

#### Scenario: A run deleted after its grant

- **WHEN** a holder's target is deleted while it holds a place, and another ask waits
- **THEN** the place is freed and granted to the waiting ask

#### Scenario: A run deleted before its grant

- **WHEN** a waiting ask's target is deleted before the turn that would grant it a place
- **THEN** it gets no place, its ask leaves the line, and the place stays free

#### Scenario: Two holders of one target

- **WHEN** one target holds two places and is deleted
- **THEN** both places are freed, once each

#### Scenario: The last release

- **WHEN** a target's only holder releases its place
- **THEN** the pool unwatches that target

#### Scenario: A failed message of a holder

- **WHEN** a message to a holder's target fails and the pool gets a `Failed` for it
- **THEN** nothing in the pool changes

### Requirement: A node can be drained

`Drain` SHALL stop new places on a known node while it keeps its holders and its offers, and `Undrain` SHALL allow them
again and serve the line; an offer SHALL not clear the drain.

#### Scenario: Draining a node with work

- **WHEN** a node with two holders is drained and work asks for a place only it could take
- **THEN** the ask waits, the two holders keep their places, and after `Undrain` the ask is granted

### Requirement: A silent node can be declared gone

`Gone` SHALL be refused for a live or unknown node, and for a silent node SHALL remove it, free its places, and send
each of its holders one answer of kind `gone` with its grant, each in a turn of its own.

#### Scenario: A machine that will not come back

- **WHEN** a node whose status is `silent`, with two holders, is declared gone
- **THEN** both holders get one `gone` answer, the node is removed, and a later release of those grants frees nothing

#### Scenario: Gone for a live node

- **WHEN** an operator sends `Gone` for a node whose status is `live`
- **THEN** it is refused and nothing changes

#### Scenario: An unknown node

- **WHEN** an operator sends `Drain`, `Undrain` or `Gone` for a node the pool does not know
- **THEN** each is refused for good and nothing changes

#### Scenario: A TellGone from someone else

- **WHEN** an actor other than the pool sends a `TellGone`
- **THEN** it is refused for good and nothing changes

#### Scenario: One holder's target is gone

- **WHEN** a silent node with three holders is declared gone and one holder's target was deleted
- **THEN** the other two holders still get their `gone` answer
