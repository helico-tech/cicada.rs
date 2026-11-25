## ADDED Requirements

### Requirement: A host's background task starts with its node

`App::task(name, f)` SHALL start `f` with the node's `ActorSystem` and a `Stopping` signal after every `on_start`
function has finished, in `serve` before it listens and in `worker`. An `App` with a task SHALL refuse to run a worker
over HTTP, and two tasks with one name SHALL refuse the command, both with exit code 2.

#### Scenario: A worker over HTTP

- **WHEN** an `App` with a task runs `worker --server <url>`
- **THEN** it exits with code 2 before it reads the config file or sends a request

#### Scenario: After on_start

- **WHEN** an `App` has an `on_start` function that creates an actor and a task that reads it
- **THEN** the task starts after `on_start` has finished and finds the actor, and `serve` listens after the task
  started

### Requirement: An early end of a task stops the node

A task that ends, fails or panics before a stop SHALL stop the node in good order and fail the command with exit code
1 and an error that names the task. Every path that ends the command after the tasks started SHALL stop them before
the store closes.

#### Scenario: A task panics

- **WHEN** a `serve` node runs two tasks and one panics
- **THEN** the other task sees `Stopping`, the node stops, and the command fails with exit code 1 and an error naming
  the task that panicked

#### Scenario: The server fails after the tasks started

- **WHEN** `serve` cannot write `Listening on`, or its HTTP server fails, after the tasks started
- **THEN** every task sees `Stopping` and ends, or is aborted at the end of the grace period, before the store closes

#### Scenario: A task returns early

- **WHEN** a task returns `Ok` while the node runs
- **THEN** the node stops, and the command fails with exit code 1 and an error saying the task ended before the stop

### Requirement: Tasks and the node stop together

At a stop, the `Stopping` signal SHALL be set when the node's stop begins, and the tasks SHALL be waited for within the
node's shutdown grace period, then aborted with a warning; the store SHALL close after both. An error or panic of a
task after the stop SHALL be logged and SHALL not change the exit code.

#### Scenario: A stop

- **WHEN** a `worker` node with a task gets SIGTERM
- **THEN** the task sees `Stopping`, the node claims no more, the task returns, and the command ends with exit code 0

#### Scenario: A task that does not stop

- **WHEN** a task ignores `Stopping` at a stop
- **THEN** it is aborted after the node's shutdown grace period with a warning naming it, and the node stops

#### Scenario: An error after the stop

- **WHEN** a task returns `Err` after it saw `Stopping`
- **THEN** a warning names the task, and the command ends with exit code 0
