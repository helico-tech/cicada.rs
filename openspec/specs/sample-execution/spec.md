# sample-execution Specification

## Purpose
How the sample service's actors run: the original binary bootstraps its singletons and watch, and a second
binary, `cicada-sample-executor`, runs the same catalog without bootstrap, also as an HTTP worker. Each turn records
the node or worker that actually ran it. Both host the ready-made heartbeat watch, which only a caller creates and
starts.

## Requirements

### Requirement: A separate sample definition runs without bootstrap

The sample package and image SHALL provide cicada-sample-executor using the shared actor catalog and App command
lifecycle without on_start hooks. The original sample definition SHALL retain its bootstrap. App's HTTP-worker
hook rejection, target/auth resolution, registration, type selection and shutdown rules SHALL remain unchanged.

#### Scenario: HTTP execution without a database

- **GIVEN** a send-only authenticated server and an executor with an inaccessible ambient CICADA_STORE
- **WHEN** the executor runs worker with explicit --server, valid client token and selected sample types
- **THEN** it registers and executes those types over HTTP without opening the ambient store
- **AND** it creates no sample singleton or wildcard dead-letter watch and starts no HTTP listener
- **AND** committed actor state is readable from the server; the worker's EffectLog has no HTTP endpoint

#### Scenario: Bootstrap belongs to a host or caller

- **WHEN** the executor registers a workload needing pricing, payments or other sample singletons
- **THEN** registration alone does not create those actors or install watches
- **AND** a bootstrap host or API caller must explicitly supply them
- **AND** the original sample still creates its hosted singletons and dead-letter watch

#### Scenario: Existing command and error policy

- **WHEN** the executor is given an explicit store with --server, a wrong client token or invalid selected type
- **THEN** the existing App failure and exit-code rules apply
- **AND** API_TOKEN is not substituted for the worker's client token
- **AND** the original sample definition still rejects HTTP worker execution because it has on_start hooks

#### Scenario: The executor keeps the App command surface

- **WHEN** the executor runs help, version, an operator command, serve or a direct-store worker
- **THEN** the normal App command applies, with the executor's version name
- **AND** its serve and direct-store worker modes also omit bootstrap
- **AND** serve retains the same sample routes and API_TOKEN protection as the original sample
- **AND** cargo run without --bin still selects cicada-sample-service

### Requirement: Sample records use the actual executor identity

The sample E2E actors and EffectLog SHALL record Turn::node_id() rather than a constructor-captured id. Sample
startup SHALL accept App's normal node-id precedence. Existing message/state shapes and contract text SHALL remain
unchanged. Sample and Catalog constructors SHALL retain source-compatible signatures; legacy identity arguments
SHALL be ignored in favor of the execution identity.

#### Scenario: A flag overrides the environment locally

- **GIVEN** CICADA_NODE_ID names one id and --node-id names another
- **WHEN** the original sample runs a Slow, Counter or Recorder turn on the store
- **THEN** startup succeeds and its durable records and EffectLog use the flag's actual execution id
- **AND** neither sample definition sets a host node_config identity override

#### Scenario: Existing constructor calls still compile

- **WHEN** a caller supplies a legacy node-id argument to Catalog or an E2E actor constructor
- **THEN** the constructor signature remains usable
- **AND** records and effects use the actual turn owner even when that argument names another id

#### Scenario: Remote worker identity is distinct from the server

- **GIVEN** an HTTP worker with a node-id flag distinct from its environment and the server id
- **WHEN** it executes sample E2E actors through the server
- **THEN** their records identify the worker lease owner selected by App
- **AND** existing state and message contract snapshots remain byte-for-byte unchanged

### Requirement: The sample image supports both entry points

The existing image SHALL contain both sample binaries and retain its default server entry point and health check.
The documented HTTP-worker invocation SHALL override the entry point, explicitly select --server and disable the
server health check and supply its token through CICADA_TOKEN. Existing App signal handling SHALL drain the worker
and return its normal exit status.

#### Scenario: Container worker acceptance

- **WHEN** Docker E2E runs the executor entry point with worker --server, CICADA_TOKEN and no supplied database volume
- **THEN** it registers selected types and runs a counter under its configured worker id
- **AND** the fresh send-only server has no sample singletons or wildcard watch from worker startup
- **AND** SIGTERM shuts the worker down with exit code zero
- **AND** the test removes its containers, anonymous image volumes and network even after failure

#### Scenario: Deployment guidance is checked

- **WHEN** the change lands
- **THEN** App, operations and E2E guidance explains bootstrap, token roles, store precedence and health-check override
- **AND** the obsolete sample node-id restriction is removed by the docs owner
- **AND** docs, doctests and consumer skill routes continue passing

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
