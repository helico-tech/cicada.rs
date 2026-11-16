# pinned-process-example Specification

## Purpose
A worked example of a pinned actor whose host starts, observes and stops a local child process: the live handle
stays in the host, the actor keeps desired and observed state, and a lost commit, a stale exit or a restarted host
never starts a second process but leaves an unknown run for explicit reconciliation.

## Requirements

### Requirement: A worked pinned actor represents one process run

The example SHALL provide an actor module, runnable binary and tests. A fresh never-reused actor key SHALL identify
one run of the fixed inert child program. It SHALL be created directly with a pin to a stable node id and initial
host_epoch. Desired/observed state SHALL be durable; Child handles SHALL belong to the host.

#### Scenario: Start and observe a run

- **WHEN** Start is handled by the matching node id and host epoch
- **THEN** the host reserves a bounded slot before spawning and starts the child at most once for that run
- **AND** repeated Start refers to the same slot regardless of message id
- **AND** natural exit is recorded with its available exit code and does not start another process
- **AND** a PID is diagnostic and is never used as restart identity

#### Scenario: Stop or complete a run

- **WHEN** Stop arrives before spawn, during spawn or while running
- **THEN** the host records sticky stop intent, prevents a later first spawn if it has not occurred, or stops/reaps
  the owned child if it has, without creating a replacement
- **AND** terminal observations are absorbing and further Start is refused
- **AND** Stop-before-Start reserves a slot/tombstone even if its actor commit is later lost
- **AND** deleting the actor is not an OS cleanup primitive

### Requirement: Host slots retain repeat evidence across lost commits

ProcessHost SHALL own at most 32 slots with a fresh host epoch and per-slot observation revisions. Reservations and
terminal tombstones SHALL be retained until shutdown. Handler cancellation SHALL NOT erase a reservation or cancel
the owned process task. A slot SHALL never spawn twice, including after an OS effect followed by a lost actor commit.

#### Scenario: A handler's successful spawn is replayed

- **WHEN** spawn occurs, its actor turn does not commit and a later claim runs against the same host table
- **THEN** the later handler observes the existing running or terminal slot and the spawn count remains one
- **AND** a failed spawn is retained as terminal rather than retried with a new OS action

#### Scenario: Capacity is exhausted

- **WHEN** 32 slots including terminal tombstones have been reserved in one host epoch
- **THEN** a new run is refused visibly without a spawn or tombstone eviction
- **AND** command/result timeout remains an unknown outcome and never permits a second spawn

### Requirement: A new host epoch requires reconciliation of old runs

The stable claim-owner id SHALL be distinct from a fresh host_epoch per host-process start. An old epoch SHALL
cause NeedsReconciliation for a nonterminal run with no OS action; confirmed terminal state SHALL remain terminal. The example SHALL NOT automatically adopt, kill or respawn old runs.

#### Scenario: Restart the host at the same node id

- **WHEN** a new ProcessHost has the same node id and a new epoch
- **THEN** startup pages through this example's actors pinned to that id and sends Refresh
- **AND** old nonterminal actors expose NeedsReconciliation even when their last stored state was Idle
- **AND** old Start/Stop/Refresh causes no spawn, PID signal or adoption
- **AND** creating a new replacement run requires explicit operator action after external verification/cleanup

#### Scenario: Clear an uncertain run after outside verification

- **WHEN** an operator independently verifies or cleans up the old resource of a NeedsReconciliation actor
- **THEN** the existing delete API may remove that actor without requiring an impossible confirmed-terminal transition
- **AND** deletion does not itself prove or perform cleanup, and a replacement requires an explicitly new key
- **AND** the example adds no Abandon message or automatic retry of that run

#### Scenario: Repeat or reorder observations

- **WHEN** Observed carries an old host epoch or a non-increasing revision
- **THEN** it is ignored without regressing state
- **AND** Running after a terminal observation is ignored and observations never change desired intent
- **WHEN** an original host sends an observation after the actor entered NeedsReconciliation
- **THEN** it is ignored even if it reports a confirmed exit with a newer revision
- **AND** a duplicate-id host may mark another host's live run uncertain, without granting cleanup authority

### Requirement: Host lifecycle and delivery are bounded and explicit

The host SHALL inject ProcessHost before building ActorSystem and attach its event pump afterwards. Each slot SHALL
have at most one pending latest observation with a stable send id per revision. Ordinary observation messages SHALL
be retried on unknown delivery outcomes without assuming transport acceptance proves handling.

#### Scenario: Observation submission fails

- **WHEN** submission fails or its acknowledgment is lost
- **THEN** the pump retries the same run/revision id once per second, with a five-second call bound
- **AND** latest terminal state subsumes older Running observations without an unbounded queue
- **AND** terminal evidence remains in the host even after an absent actor or accepted submission ends retries

#### Scenario: Shutdown cannot confirm exit

- **WHEN** host admission is closed and shutdown stops/reaps live slots and flushes observations within its
  five-second cleanup budget while ActorSystem, pump and store remain open
- **THEN** successful cleanup requires confirmed exit, not just a kill request
- **AND** unconfirmed cleanup is reported and the demo exits nonzero without claiming Stopped
- **AND** ActorSystem then stops with bounded configured grace, the pump is stopped/joined, and the store closes last
- **AND** failed observation flush is reported rather than treated as durable completion
- **AND** kill-on-drop is only best effort and abrupt host death may leave an orphan

### Requirement: The example tests application limits without strengthening core guarantees

The runnable adapter SHALL execute only its own inert child mode without a shell, arbitrary command or descendants.
Actor/protocol tests SHALL use TestActors and virtual time. A separate bounded subprocess smoke test SHALL cover real
spawn/normal-exit/stop/reap behavior. Documentation SHALL distinguish host-lifetime repeat safety from crash recovery.

#### Scenario: Check race and restart behavior

- **WHEN** the example is checked
- **THEN** host construction supplies the child path, never user input; OS tests run on Linux/macOS while other
  platforms retain protocol tests without OS verification claims
- **AND** tests cover lost commit replay through public TurnSource, capacity, sticky stop/spawn races, stale events,
  send uncertainty, deletion after external verification, late original-host exit after NeedsReconciliation, stalled
  operations and shutdown, plus a new-epoch zero-action assertion
- **AND** tests/pinned_process_os.rs runs the same OS adapter in a bounded subprocess helper using fixed child
  fixtures/handshakes and owned-handle cleanup, as part of cargo test; cargo check --examples checks the demo
- **AND** old portable fixtures and core wire/schema contracts remain unchanged

#### Scenario: Explain the example to consumers

- **WHEN** the implementation is handed off
- **THEN** both consumer skills route to the example and the docs owner receives outside-effects, lifecycle,
  websocket comparison and cleanup guidance under existing guide/operations headings
- **AND** no test claims generic exactly-once OS effects, PID adoption or guaranteed abrupt-death cleanup
- **AND** protected prose needed for docs/doctest/skill checks is prepared by the docs owner before landing
