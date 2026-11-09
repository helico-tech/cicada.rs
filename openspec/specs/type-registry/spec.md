# type-registry Specification

## Purpose
TBD - created by archiving change remove-registered-type. Update Purpose after archive.

## Requirements

### Requirement: A type's registration can be removed

`DELETE /types/{name}` SHALL remove the registration of the type and answer `204`. It SHALL answer `404` with the problem
type `no-such-type` when no type has that name, and `409` with the problem type `type-in-use` while actors of the type
exist, leaving the registration as it was. The check and the removal SHALL be one atomic step. It SHALL be a write route:
the read-only router does not serve it. The type SHALL NOT be served after its removal, by lookups or by creates.

#### Scenario: An unused type

- **WHEN** no actor of the type `counter` exists and a client deletes `/types/counter`
- **THEN** the answer is `204`, `GET /types/counter` answers `404`, and a create of a `counter` actor answers `404`

#### Scenario: A type in use

- **WHEN** an actor `counter/a` exists and a client deletes `/types/counter`
- **THEN** the answer is `409 type-in-use` and the type stays registered

#### Scenario: An unknown type

- **WHEN** a client deletes `/types/nope`
- **THEN** the answer is `404 no-such-type`

#### Scenario: A create races the removal

- **WHEN** an actor is created while its type is removed
- **THEN** either the create finishes first and the removal is refused with `409`, or the create finds the type gone and
  answers `404`; no actor stays behind without its type

#### Scenario: Registering again

- **WHEN** a type was removed and a node that hosts it starts
- **THEN** the node registers its contract again
