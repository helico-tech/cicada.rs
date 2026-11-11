# Spec Delta

## MODIFIED Requirements

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
