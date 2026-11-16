# node-down-alerts Specification

## Purpose
An optional ready-made monitor that samples the node table and publishes each observed heartbeat silence of two
minutes or more once through a Topic. It is sampled observability: it never proves a machine failed and grants no
authority to claim, move or fence work.

## Requirements

### Requirement: An optional singleton monitor reads native node rows

Cicada-actors SHALL provide NodeDownMonitor at cicada.node-down/_ with fixed Topic destination and injected read-only
NodeSnapshotSource. ActorSystem SHALL implement the source using nodes then store now. The monitor SHALL be explicitly
created unpinned and started with Rearm, without a NodeConfig flag, automatic activation or node-table changes.

#### Scenario: Construct without a circular dependency

- **WHEN** a host creates an unstarted reader ActorSystem on the same Store and supplies it to the monitor actor
- **THEN** the hosting system can be built separately, and the reader adds no heartbeat/node row
- **AND** every host of the singleton must use the same source store
- **AND** hosts explicitly create its Topic and register subscribers through the existing Topic protocol

### Requirement: Polls preserve bounded state and recoverable observation failures

State SHALL contain topic, at most 1000 alerted node watermarks, paired armed/nextPollAt and optional lastCheckedAt/
lastProblem. Reads SHALL have a five-second bound. Source errors, timeout, invalid samples or more than 1000 rows
SHALL preserve watermarks, emit no alerts, record a bounded problem category and arm the next ten-second poll.

#### Scenario: Validate construction and snapshots

- **WHEN** state or a source snapshot is consumed
- **THEN** typed construction/decoding validates topic, timer pairing, map bound and representable timestamps
- **AND** snapshot checks reject duplicate ids, invalid ids/details, unordered startedAt/lastSeen/as_of,
  regressing as_of and regressing lastSeen for the same row identity
- **AND** one invalid row or more than 1000 rows suppresses every alert for the sample while polling continues
- **AND** the row cap bounds persisted work rather than the full node API's transient read/allocation
- **AND** repeated read timeouts can suppress alerts indefinitely and remain visible in lastProblem

#### Scenario: Read fails then recovers

- **WHEN** the source is unavailable, times out or returns excessive/invalid rows
- **THEN** the check records a problem category without raw error text and commits only its next timer/status changes
- **WHEN** a later read succeeds
- **THEN** detection resumes against preserved watermarks without inventing recovery events

### Requirement: Guarded polling and Rearm use durable self messages

Poll SHALL act only when self-sent with the current armed message id at or after nextPollAt. Each valid check SHALL
replace its timer for ten seconds after turn.current_time with the existing ready-made retry policy. Public Rearm
SHALL preserve watermarks and replace the timer at its stored deadline, or now on first start.

#### Scenario: Ignore or defer an invalid poll

- **WHEN** a poll is external, stale or redriven under another id
- **THEN** it does not read the source or change state
- **WHEN** the armed poll is delivered early
- **THEN** it replaces its timer at the same deadline without reading nodes

#### Scenario: Recover a lost timer

- **WHEN** a timer was removed, failed or purged and the operator sends Rearm
- **THEN** the old timer is cancelled where applicable and a replacement is armed at the existing deadline
- **AND** alerted watermarks remain intact, so this does not itself permit repeated alerts

### Requirement: Observed dead episodes publish once through atomic actor effects

A row SHALL qualify as dead when snapshot.as_of minus lastSeen is at least 120 seconds. A newly observed dead
watermark SHALL cause one NodeDown Topic Publish per monitor, atomically with its stored watermark and next poll.
Leases and commits SHALL coordinate competing monitor hosts without a new store election mechanism.

#### Scenario: First and repeated dead observation

- **WHEN** a dead row is first seen, including one never observed alive
- **THEN** one Publish and its id/startedAt/lastSeen watermark commit together
- **WHEN** the same dead watermark is seen again
- **THEN** it produces no new alert
- **AND** readings just below 120 seconds do not alert and readings at 120 seconds do

#### Scenario: A new observed episode

- **WHEN** startedAt changes or lastSeen advances and the sampled row is dead
- **THEN** one new alert may be committed for that watermark without requiring an intermediate alive observation
- **WHEN** a successful snapshot shows the row absent or non-dead
- **THEN** its previous watermark is removed without a recovery notification
- **AND** deleting/recreating the monitor explicitly resets history and may alert again

#### Scenario: Competing hosts or failed publication

- **WHEN** two hosts poll the singleton or a turn is replayed after a lost commit
- **THEN** ordinary fencing and atomic state/sends permit only one committed Publish for the observed watermark
- **WHEN** the configured Topic cannot accept the send or disappears before commit
- **THEN** the whole check has normal handler/commit failure semantics, with no partial marker or alert publication
- **AND** after repairing the destination the operator sends Rearm if its poll was lost or terminally failed

### Requirement: NodeDown reports sampled evidence using the existing Topic contract

NodeDown SHALL contain node, startedAt, lastSeen, detectedAt and optional hostname/cicadaVersion/label, with camelCase
JSON and store-instant strings. detectedAt SHALL mean source snapshot as_of. Publications SHALL be ordinary messages,
with existing Topic subscriber/retry/delivery semantics and no core notification flag or recovery event.

#### Scenario: Deliver to configured subscribers

- **WHEN** the monitor publishes its private NodeDown envelope through TopicMsg::Publish
- **THEN** receiver enums can declare NodeDown(cicada_actors::NodeDown), and topic membership at processing time governs delivery
- **AND** absent details are omitted and no hosted-type or running-turn list is embedded
- **AND** successful enqueue does not claim successful paging/email or subscriber handling

#### Scenario: Limits of silence observations

- **WHEN** a heartbeat or clean removal races a sampled dead row
- **THEN** an alert may describe an already-stale lastSeen; no atomic recheck against the heartbeat is promised
- **WHEN** rows are removed by clean stop or existing daily pruning before the monitor reads them
- **THEN** no death history can be inferred from their absence
- **AND** all-cluster outage has no observer until a store and polling monitor return
- **AND** duplicate node ids and row timestamps do not establish host incarnation or physical-machine failure
- **AND** row-free HTTP workers remain unobserved and no alert automatically reassigns pins or fences outside work

### Requirement: Contracts and examples make the observability boundary explicit

Tests SHALL pin the new monitor/event formats and exercise polling, deduplication, failure and Topic integration.
The change SHALL provide a checked node_down example and docs handoff, preserving existing core/actor contracts,
portable fixture cases and node-schema behavior.

#### Scenario: Verify and document the monitor

- **WHEN** implementation is prepared for landing
- **THEN** virtual-time tests include threshold boundaries, episodes between polls, source/size/shape failures,
  stale/early timers, Rearm, Topic rollback/repair, competing hosts and lost-commit replay
- **AND** actual node-row integration verifies reader behavior without new heartbeat registration
- **AND** wire_formats.rs checks the monitor contract and NodeDown receiver samples and generates its reference
- **AND** both consumer skills route to the new guide heading/reference/example after exact docs-owner integration
- **AND** the example watches monitor Failed/Stopped and separately inspects lastProblem for caught read failures
- **AND** operational guidance names whole-sample refusal, the row cap/read cost, startup dead-row bursts, unlimited
  flapping alerts and pruning/race limits while distinguishing observations from claim authority
