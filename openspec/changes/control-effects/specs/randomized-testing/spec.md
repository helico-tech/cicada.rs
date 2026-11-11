# Spec Delta

## ADDED Requirements

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
