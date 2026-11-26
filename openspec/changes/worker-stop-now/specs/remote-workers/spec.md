## ADDED Requirements

### Requirement: A stop's grace period can end early

`Worker::end_grace` SHALL end the grace period of a stop that has begun: running turns SHALL be cancelled as at the end
of the grace period, and the stop SHALL return within the cancel timeout. Before a stop begins it SHALL do nothing and
SHALL NOT shorten a later stop. A worker over HTTP SHALL give its host a `StopNow` handle in `WorkerReady` that does the
same.

#### Scenario: Ending the grace period during a stop

- **WHEN** a turn runs, the worker's stop begins with a grace period of 30 seconds, and `end_grace` is called after 1
  second
- **THEN** the turn is cancelled at once and the stop returns within the cancel timeout

#### Scenario: Before a stop

- **WHEN** `end_grace` is called while the worker runs, and the worker is stopped later
- **THEN** the later stop gives its running turns the full grace period
