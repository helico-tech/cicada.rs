## ADDED Requirements

### Requirement: A run can be started with a reply route

`FlowMsg::Start` SHALL carry a reply route; the run SHALL save the first one, and a later `Start` with another route
SHALL fail for good. On a running or cancelling run it SHALL then act as a host `Continue`. On an ended run without a
saved route it SHALL save the route and send the outcome in the same turn.

#### Scenario: A flow starts and awaits a child run

- **WHEN** a flow calls `request_flow` with a child target of a run whose output is `7`, and waits for it
- **THEN** the run is created and started in the caller's commit, and the wait returns `FlowDone` with output `7`

#### Scenario: A second caller

- **WHEN** a run has a saved route and a `Start` with another route arrives
- **THEN** that message fails for good and the saved route stays

#### Scenario: A late start

- **WHEN** a run is done without a route and a `Start` with an actor route arrives
- **THEN** that turn sends `FlowDone` with the saved output to the route

### Requirement: An ended run replies its outcome in a turn of its own

The turn that ends a run with a saved route SHALL arm a `Continue`. Any `Continue` that reaches an ended run whose
route has not been replied to SHALL send `FlowDone`, `FlowCancelled` or `FlowFailed` with the run's address through the
route, mark it replied and clear `armed`. A failed send SHALL leave the run ended. A run without a route SHALL send
nothing.

#### Scenario: An actor caller

- **WHEN** an actor starts a run with an actor route with correlation id `c1`, and the run fails its main code
- **THEN** after the cleanup the actor receives `FlowFailed` with the run and the reason, with correlation id `c1`

#### Scenario: A deleted caller

- **WHEN** an actor caller is deleted before its run ends
- **THEN** the run still ends, and only the outcome's message fails

#### Scenario: A lost outcome sent again

- **WHEN** an ended run's outcome `Continue` was purged before it ran, and a public `Continue` arrives
- **THEN** that turn sends the outcome and marks the route replied
