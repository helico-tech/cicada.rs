## ADDED Requirements

### Requirement: A typed watcher reads a failure notice for a payload at the limit

A failed message with an accepted payload of depth 64 SHALL produce a readable Failed notice of depth 65. Every
backend SHALL retain and claim the notice's original payload, and a typed watcher SHALL decode it both locally and
through an HTTP remote worker, on ordinary runtime threads in a debug build. Keeping that payload in the watcher's
state SHALL remain subject to the ordinary 64-level state limit; reading a notice does not grant a state exception.

#### Scenario: Store and local typed watcher

- **WHEN** a depth-64 message fails under a watch on memory, SQLite or PostgreSQL
- **THEN** the store writes and claims its complete Failed notice
- **AND** a local typed watcher compares its payload through a probe without copying it into state and completes normally

#### Scenario: HTTP worker watcher

- **WHEN** that notice is leased over HTTP to a Worker hosting the typed watcher
- **THEN** the lease decodes, the handler sees the original payload and the notice turn completes normally

#### Scenario: Retaining the notice in an ordinary state

- **WHEN** a watcher instead copies the depth-65 notice into its own state and returns
- **THEN** the turn fails at state validation rather than at message decoding, with no new depth exemption
