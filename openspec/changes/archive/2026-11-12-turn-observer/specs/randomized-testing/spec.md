# randomized-testing

## ADDED Requirements

### Requirement: The randomised tests cover lost claims and the claim limit

The comparison of the stores SHALL compare the `lostClaims` of every message, also after a claim of an actor whose lease
ran out. The simulation of nodes under faults SHALL run its nodes with a limit of 2 lost claims, and its checks
SHALL accept a message that ends `failed` with the error of the limit only when at least two earlier claims of it
were replaced by another claim without a commit or a release, and no handler ran for the claim that failed it.

#### Scenario: A lost claim in the comparison

- **WHEN** a generated list of operations claims a message, lets the lease run out and claims the actor again
- **THEN** the in-memory store and SQLite answer the same `lostClaims` for that message

#### Scenario: A scenario that reaches the limit

- **WHEN** a generated scenario makes two claims of one message lose their lease
- **THEN** the third claim ends the message `failed` with the error of the limit, and the checks pass

#### Scenario: A failure by the limit that is not allowed

- **WHEN** a log holds a message that ended `failed` with the error of the limit after one lost claim
- **THEN** the checks report it
