## ADDED Requirements

### Requirement: A stop's grace period can end early

`Worker::end_grace` SHALL end the grace period of a stop that has begun: running turns SHALL be cancelled as at the end
of the grace period, and the stop SHALL return within the cancel timeout. Before a stop begins it SHALL do nothing and
SHALL NOT shorten a later stop. This is the one way to shorten the drain of "Worker shutdown preserves handler-end and
cleanup boundaries".

#### Scenario: Ending the grace period during a stop

- **WHEN** a turn runs, the worker's stop begins with a grace period of 30 seconds, and `end_grace` is called after 1
  second
- **THEN** the turn is cancelled at once and the stop returns within the cancel timeout

#### Scenario: Before a stop

- **WHEN** `end_grace` is called while the worker runs, and the worker is stopped later
- **THEN** the later stop gives its running turns the full grace period

### Requirement: A worker over HTTP hands its host Stop Now

A worker over HTTP SHALL give its host a `StopNow` handle in `WorkerReady`. Its `now` SHALL end the grace period of the
worker's stop, and a request that comes before the worker's stop has begun SHALL be kept until it does.

#### Scenario: Stop Now right after the stop signal

- **WHEN** the host signals the stop of a worker over HTTP and calls `StopNow::now` before the worker has begun stopping
- **THEN** the worker's grace period ends as soon as its stop begins
