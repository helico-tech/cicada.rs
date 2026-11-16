## ADDED Requirements

### Requirement: The shared sample catalog hosts the existing heartbeat watch

The sample service and executor SHALL include HeartbeatWatchActor under cicada.heartbeat-watch through their shared
catalog. App's all and exact-name selections SHALL host its existing contract, while none SHALL exclude it. Startup
SHALL NOT create a heartbeat-watch instance or send Beat. Native node monitoring and NodeDownMonitor hosting SHALL
remain outside this catalog addition.

#### Scenario: Both execution definitions select the same type

- **WHEN** either sample definition selects all or cicada.heartbeat-watch
- **THEN** the selected types include the existing HeartbeatWatchActor contract
- **AND** selecting none excludes it
- **AND** the type has the same messages, state, text and hash as the ready-made actor

#### Scenario: Hosting alone does not start a watch

- **WHEN** either sample definition starts with heartbeat-watch selected against a fresh store
- **THEN** no heartbeat-watch actor, Beat or Expire timer is created automatically
- **AND** the original sample's unrelated singleton and dead-letter bootstrap policy stays unchanged
- **AND** HTTP executors gain no native node-table heartbeat

#### Scenario: A caller explicitly starts a round

- **GIVEN** a caller-created alarm target and a valid named watch with positive allowed silence
- **WHEN** the watch is created and then receives Beat through the sample API
- **THEN** its initial waiting state has no armed timer
- **AND** the actor state and GET /actors/{type}/{key}/messages show waiting and no pending messages before Beat
- **AND** handling Beat persists alive, lastBeatAt and an armed Expire using the existing actor behavior
- **AND** the pending message read contains the matching Expire after Beat commits
- **AND** the smoke verifies direct-store sample hosting and the HTTP executor in process, without a new Docker test

### Requirement: Catalog expansion preserves existing contracts and checked guidance

The sample SHALL pin the existing heartbeat-watch contract in its contract snapshots and text digests without
changing previous entries or the ready-made contract reference. The docs session SHALL document sample availability
and explicit creation/Beat, linking to the existing heartbeat-watch guide. Existing deadline/alarm semantics SHALL
continue to be verified by the ready-made actor's virtual-time tests.

#### Scenario: Contract regression checks

- **WHEN** the sample contract snapshots and text digests are regenerated
- **THEN** only the heartbeat-watch contract and digest entry are added
- **AND** the new snapshot's parsed contract JSON matches the existing ready-made reference
- **AND** existing sample contracts and the ready-made heartbeat reference remain unchanged

#### Scenario: Guidance and timing verification

- **WHEN** the catalog change lands
- **THEN** the App sample guidance identifies heartbeat-watch hosting without promising automatic instances
- **AND** its handoff builds on the predecessor sample-worker-mode's landed prose
- **AND** the existing virtual-time waiting/first-Beat and one-alarm-after-silence tests still pass
- **AND** docs, doctests and consumer skill routes pass
