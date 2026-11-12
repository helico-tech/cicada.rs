# http-schema-check

## MODIFIED Requirements

### Requirement: A repeated send is answered before it is checked

A send that names the id of a message that is pending, or whose outcome is still shown, SHALL NOT be checked
against the schema. It SHALL get the answer of a repeated send. The limits of an envelope and of a value SHALL be
checked before the repeated id, so a repeat beyond a limit SHALL be refused with `400`.

#### Scenario: A repeat of a message that does not match

- **WHEN** code sends a message with the id `m1` as JSON that does not match the schema, on a node that does not
  host the type, and a client then sends `m1` again over HTTP with the same payload while it is pending
- **THEN** the answer is `200`, as for every repeated send

#### Scenario: A repeat of a message whose outcome is hidden

- **WHEN** the same message has ended and its retention hides the outcome, and the client sends `m1` again
- **THEN** the answer is `422 invalid-payload`: the payload is checked before the store refuses the id

#### Scenario: A send without an id

- **WHEN** a client sends a payload that does not match, without an id
- **THEN** the answer is `422 invalid-payload`

#### Scenario: A repeat beyond the depth limit

- **WHEN** a `Failed` notification of a payload 64 levels deep, which the store writes 65 levels deep, is pending, and
  a client sends a message with its id, its type and its payload
- **THEN** the answer is `400`, not the answer of a repeated send

### Requirement: A refusal by the schema costs time and memory in proportion to the size of the request

The check of a payload or a state SHALL ask only whether the value matches, and nothing SHALL run after a refusal. A
refusal by the schema SHALL cost time and memory in proportion to the size of the request, not to the number of places
that do not match, nor to the number of shapes the value could have at each level.

#### Scenario: A request with many mismatches

- **WHEN** a client sends a payload, or creates an actor with a state, of about 1 MiB with 520,000 values of the wrong
  kind, also inside an optional part, or 64 levels of a type that refers to itself with wrong values below the first
  level, to a node that hosts the type and to one that does not
- **THEN** the answer is `422` within seconds, and the check adds at most a few times the size of the body to what the
  request holds

#### Scenario: A small nested value with one mistake

- **WHEN** a client sends a payload, or creates an actor with a state, that is an expression 30 deep of an enum whose
  variants share the names of their fields, or a tree 24 deep of an untagged enum whose variants share a field, with
  one wrong leaf, to a node that hosts the type and to one that does not
- **THEN** the answer is `422` at once on both

## REMOVED Requirements

### Requirement: A value at the deepest level the parser allows is checked like any other

**Reason**: a payload or a state deeper than 64 levels is refused with `400` before the schema check
(`value-depth-limit`), so a value of 126 levels no longer reaches it.

**Migration**: the requirement "A value at the depth limit is checked like any other" takes its place.

## ADDED Requirements

### Requirement: A value at the depth limit is checked like any other

A payload or a state 64 levels deep SHALL be checked against its schema like any other value, on every node, and the
check SHALL NOT end the process. The schema check SHALL keep running on a thread of its own, as a schema can be deep.

#### Scenario: A value at the depth limit

- **WHEN** a client sends a payload of 64 levels of a type that refers to itself, once matching the schema and once
  with mismatches at every level, to a node that hosts the type and to one that does not
- **THEN** both nodes answer `202` for the first, which is then handled, and `422 invalid-payload` with the same detail
  for the second, and a payload one level deeper is `400`

#### Scenario: A type with several schema steps between its levels

- **WHEN** a client sends a payload and creates an actor with a state of a type that has six newtypes between two of
  its levels, at 24 levels and at 64, once matching and once not, to a node that hosts the type and to one that does
  not, both served on a runtime whose threads have their default stack
- **THEN** both nodes answer `202` and `201` for the matching ones and `422` for the others, and the process goes on
