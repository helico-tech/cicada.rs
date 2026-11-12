# randomized-testing Specification

## Purpose
How randomised tests exercise Cicada's parsers, store backends and runtime, what they check, and how a failure is
replayed and kept as a test.

## Requirements

### Requirement: Parsers survive any input

The parsers of addresses, percent-encoded text, durations, instants and page cursors, and the decoders of HTTP request
bodies, SHALL answer any input with a value or an error. They SHALL NOT panic.

#### Scenario: Random text

- **WHEN** a parser receives random text
- **THEN** it returns a value or an error, and the test run continues

#### Scenario: Text close to the valid format

- **WHEN** a duration parser receives well-formed text with numbers near the limits of the arithmetic
- **THEN** it returns a value or an error, and does not panic

### Requirement: Formatted values parse back

A value that Cicada formats as text SHALL parse back to an equal value.

#### Scenario: Round trip

- **WHEN** a random valid address, duration, instant or page cursor is formatted and then parsed
- **THEN** the result equals the original

### Requirement: Backends agree

For the same list of store operations on the same virtual clock, the in-memory store and the SQLite store SHALL give
equivalent results and end in equivalent states. The comparison SHALL NOT depend on the order in which a backend
hands out epochs to different instances: an operation that uses a claim, or an epoch before or after it, SHALL reach
the same claim of the same instance in both backends.

#### Scenario: A random list of operations

- **WHEN** one random list of operations runs against both backends, one operation at a time
- **THEN** each operation gets an equivalent answer from both

#### Scenario: Compared after the last operation

- **WHEN** the list of operations has ended
- **THEN** the actors, the pending messages and the retained outcomes of both backends are equivalent

#### Scenario: Numbers a backend picks itself

- **WHEN** sequence numbers or epochs differ between backends in value but not in relative order
- **THEN** the results count as equivalent

#### Scenario: An error from both backends

- **WHEN** both backends refuse an operation with the same kind of store error and different text
- **THEN** the results count as equivalent

#### Scenario: Two instances are due at the same instant

- **WHEN** two instances are due at the same instant, the backends hand out their epochs in a different order, and a
  later operation commits, renews or releases with the epoch before or after a claim's own
- **THEN** both backends get a request for the same claim of the same instance, and their answers are equivalent

### Requirement: Faults are injected per node

A simulated node SHALL reach the shared store only through its own fault layer. The fault layer SHALL be able to fail a
call before it applies, fail it after it applies, delay it, and make every later call from that node fail.

#### Scenario: A lost acknowledgement

- **WHEN** the fault plan fails a commit after it applies
- **THEN** the store holds the commit and the node sees an unavailable store

#### Scenario: A node crash

- **WHEN** the fault plan makes a node dead
- **THEN** every later store call from that node fails, and the other nodes keep working

### Requirement: Guarantees hold under faults

After a simulated run, once faults have stopped and virtual time has moved on, the history of the run SHALL satisfy
G1, G3, G4, G5 and G6, as defined in `docs/architecture.md`.

#### Scenario: Turns do not overlap (G1)

- **WHEN** a run has ended
- **THEN** no handler of an instance started while another handler of that instance was still running, and at most one commit succeeded per epoch

#### Scenario: Every message ends once (G3)

- **WHEN** a run has drained
- **THEN** every accepted message has exactly one outcome

#### Scenario: Effects apply once (G4)

- **WHEN** a run has drained
- **THEN** the effect of each message appears in the state of its instance at most once, and only for a message that ended `done`

#### Scenario: Delivery order (G5)

- **WHEN** a run has ended
- **THEN** no handler saw a message before its delivery time, and turns of an instance followed `(deliverAt, seq)`

#### Scenario: No hidden retry (G6)

- **WHEN** a handler returned for a message
- **THEN** it ran again for that message only if its retry policy asked for that or a node lost that turn before commit

### Requirement: Work drains after faults stop

Once faults have stopped, every instance that is not paused SHALL work through its due messages within a bounded amount
of virtual time (S8), as long as at least one node is alive.

#### Scenario: Faults end

- **WHEN** the fault plan has ended and one node is still alive
- **THEN** no due message of an instance that is not paused is left pending after the bound

### Requirement: A failure can be replayed

A failing case SHALL be reduced to a short list of steps, reported in the test output and kept in the repository, and
SHALL fail again when the tests next run.

#### Scenario: A guarantee is broken

- **WHEN** a randomised test finds a failure
- **THEN** the output shows the reduced list of steps and which check failed

#### Scenario: The same case runs again

- **WHEN** a saved failing case runs again without a fix
- **THEN** it fails on the same check

### Requirement: Randomised tests run with the normal tests

A short run of the randomised tests SHALL be part of `cargo test --workspace` on the stable toolchain, and a longer run
SHALL be possible by setting the number of cases.

#### Scenario: The normal test run

- **WHEN** `cargo test --workspace` runs without any extra setting
- **THEN** the short run of each randomised test is included, with no database server or Docker needed

#### Scenario: A longer run

- **WHEN** a developer sets `PROPTEST_CASES` to a larger number
- **THEN** that many cases run

### Requirement: Notifications are written once under faults

The simulation of nodes under faults SHALL hold an actor that watches the others and is never removed, and SHALL
check that every `failed` outcome of a watched actor has exactly one `Failed` message for that watcher, pending or
finished, whatever commits were repeated or lost. Its check that every ended message was accepted SHALL count a
notification as accepted.

#### Scenario: A commit that is repeated

- **WHEN** the acknowledgement of a commit that ends a message as `failed` is lost, and the node commits again
- **THEN** the watcher has one `Failed` message for that message, not two

### Requirement: The comparison of backends covers watches

The comparison of the in-memory store with SQLite SHALL set and remove watches, in a commit and from outside, and
SHALL compare the stored watches and the notifications that each backend writes, with their ids and their order.

#### Scenario: A random list of operations with watches

- **WHEN** one random list of operations that sets watches, fails messages and removes actors runs against both
  backends
- **THEN** both hold the same watches, and the same notifications with the same ids in the same mailboxes

### Requirement: The randomised tests cover control effects

The comparison of the in-memory store with SQLite SHALL build commits that pause, resume, purge and delete actors of
its pool, alone and together with creates, sends and watches. The simulation of nodes under faults SHALL hold an
actor that restarts another, and its checks SHALL hold for both.

#### Scenario: A random list of operations with control effects

- **WHEN** one random list of operations whose commits delete, create, purge and pause actors runs against both
  backends
- **THEN** each operation gets an equivalent answer from both, and both end in equivalent states

#### Scenario: A restart under faults

- **WHEN** an actor of the simulation restarts another while commits are repeated or lost
- **THEN** every accepted message of both still ends once, and no effect is applied twice
