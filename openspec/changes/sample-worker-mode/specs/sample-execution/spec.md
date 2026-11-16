## ADDED Requirements

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
