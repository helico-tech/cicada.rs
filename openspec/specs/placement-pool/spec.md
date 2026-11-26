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

### Requirement: A member offers a node's places while it can run work

`PlacementMember::run` SHALL, every interval until stopped, call the host's probe and send an `Offer` with the node's
own id (`ActorSystem::node_id`) and its identity's instance when the probe gives a capacity, and send nothing when it
gives none; a failed send SHALL be tried in the next round, and `run` SHALL end on its stop also while a probe hangs.

#### Scenario: A runtime that stops answering

- **WHEN** the probe returns a capacity for three rounds and then nothing
- **THEN** the pool gets three offers, then none, and marks the node silent after its silence time

#### Scenario: A node id from the command line

- **WHEN** the node runs with `--node-id` set to another id than the identity file's
- **THEN** the member offers the node's own id, the one its pins use

#### Scenario: A zero interval

- **WHEN** a host builds `MemberSettings` with an interval of zero
- **THEN** it is refused

### Requirement: A node keeps its identity across restarts

`NodeIdentity::load_or_create` SHALL give the same node id and instance across restarts, SHALL create a missing folder
and file, and SHALL fail with `InvalidIdentity` for a file it cannot read or parse, never making a new identity then.

#### Scenario: A restart

- **WHEN** a node's process restarts and loads its identity file again
- **THEN** it gets the same node id and instance

#### Scenario: A corrupt identity file

- **WHEN** the identity file holds text that is not the identity JSON
- **THEN** `load_or_create` fails with `InvalidIdentity` and leaves the file as it is

### Requirement: A member notices another machine under its node id

Each round, before its probe, `PlacementMember` SHALL read its own node row and, when its host name is set and differs
from the one its node writes (`ActorSystem::hostname()`), SHALL send no offer that round and send the pool `Duplicate {
node, host }`; the pool SHALL mark the node in conflict with that host, send one `conflict` alert per mark, give the
node no new places while marked, and clear the mark one silence time after the last report.

#### Scenario: A copied identity folder

- **WHEN** two machines run with the same identity and the node row's host name flips between them
- **THEN** each member that sees the other host skips its offers and reports it, and the pool alerts once

#### Scenario: A redeploy

- **WHEN** a node is replaced by a new container with another host name and the old one has stopped
- **THEN** no duplicate is reported

### Requirement: One process per identity folder

`NodeIdentity::load_or_create` SHALL take an exclusive lock on the identity folder before it reads or creates the file
and keep it while the identity or a clone of it lives; a second process SHALL fail with the kind `Locked`.

#### Scenario: A second process

- **WHEN** a node's member holds its identity and a second process loads the same folder
- **THEN** the second load fails with the kind `Locked`

### Requirement: A node's host name can be set

`NodeConfig::hostname` SHALL set the host name a node writes to its row, the machine's own when unset, and
`ActorSystem::hostname()` SHALL return the value the node writes.

#### Scenario: A container with a stable name

- **WHEN** a node starts with `NodeConfig::hostname` set to `runner-1`
- **THEN** its node row's host name is `runner-1` and `ActorSystem::hostname()` returns it

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

### Requirement: An operator changes a pool's settings

A `Configure` message SHALL change any of a pool's silence, node limit, line limit and holder limit, within the bounds of
its create, and keep the others. A message with no setting or with a value out of bounds SHALL be refused for good and
change nothing. A limit lowered below what the pool has SHALL remove nothing and SHALL keep new nodes, holders or
waiting asks out until the count is below it. A new silence SHALL re-arm the silence check at the new earliest end.

#### Scenario: A lower holder limit

- **WHEN** a pool with three holders gets `maxHolders` 2 and then an ask that a node has room for
- **THEN** the three holders stay, the ask is queued, and it is granted only after two holders release

#### Scenario: A higher holder limit

- **WHEN** a pool at its holder limit with a waiting ask gets a higher `maxHolders`
- **THEN** the waiting ask is granted in that turn

#### Scenario: A shorter silence

- **WHEN** a live node last offered 50 seconds ago and the pool's silence changes from 60 to 30 seconds
- **THEN** the node is marked silent in the next turn and a `silent` alert goes out

#### Scenario: A message without a setting

- **WHEN** a `Configure` with no field set, or with `maxHolders` 0, reaches the pool
- **THEN** it is refused for good and the state is unchanged

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

#### Scenario: A held key in a full pool

- **WHEN** a full pool gets an offer from a new node with a resource key another live node holds
- **THEN** the offer is recorded as a contest with its `conflict` alert, and no `full` notice goes out

#### Scenario: A known node in a full pool

- **WHEN** a known node of a full pool offers
- **THEN** its entry is renewed as before
