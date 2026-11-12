# http-schema-check Specification

## Purpose
Every node checks the payload of a send and the state of a create over HTTP against the JSON Schema that the
type registered, whether or not it hosts the type, and refuses what does not match. The check costs time and
memory in proportion to the request, and a host can test its samples against the schema and the decoder.

## Requirements

### Requirement: A send over HTTP must match the schema of its message type

`POST /actors/{type}/{key}/messages` SHALL check the payload against the JSON Schema that the registered contract of
the target's type holds for the message type, on every node, whether or not the node hosts the type. A payload that
does not match SHALL be refused with `422` and the problem type `invalid-payload`, and no message SHALL be stored.

#### Scenario: A node that does not host the type

- **WHEN** a client sends a message of the type `Add` whose payload has a field of the wrong kind, to a node that
  does not host the target's type
- **THEN** the answer is `422 invalid-payload`, and no message is stored

#### Scenario: A node that hosts the type

- **WHEN** the same send goes to a node that hosts the type
- **THEN** the answer is `422 invalid-payload`, with the same detail

#### Scenario: A payload that matches

- **WHEN** a client sends a message whose payload matches the schema, to a node that does not host the type
- **THEN** the answer is `202`

#### Scenario: A field that the type does not know

- **WHEN** a client sends a payload with an extra field to a type whose message denies unknown fields
- **THEN** the answer is `422 invalid-payload`

#### Scenario: A required field is missing

- **WHEN** a client sends a payload without a field that the schema requires
- **THEN** the answer is `422 invalid-payload`, with the same detail on every node

#### Scenario: A message type the contract does not have

- **WHEN** a client sends a message type that the contract does not have
- **THEN** the answer is `422 unknown-message-type`, as before

### Requirement: The state of a create over HTTP must match the state schema

`PUT /actors/{type}/{key}` SHALL check the state against the state schema of the registered contract, on every node.
A state that does not match SHALL be refused with `422` and the problem type `invalid-state`, and no actor SHALL be
created.

#### Scenario: A node that does not host the type

- **WHEN** a client creates an actor with a state that has a field of the wrong kind, on a node that does not host
  the type
- **THEN** the answer is `422 invalid-state`, and no actor exists

#### Scenario: A state that matches

- **WHEN** a client creates an actor with a state that matches the schema, on a node that does not host the type
- **THEN** the answer is `201`

#### Scenario: The actor exists

- **WHEN** a client creates an actor that exists, with a state that does not match the schema
- **THEN** the answer is `422 invalid-state`, and the actor is as it was

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

### Requirement: A hosting node also decodes, with the same answer

A node that hosts the type SHALL decode a payload or a state that the schema accepts, as today, and SHALL refuse one
that does not decode with `422` and the same problem type. After a refusal by the schema it SHALL run no decoder. The
detail SHALL say which of the two checks refused the value.

#### Scenario: A payload that the schema allows and the type cannot decode

- **WHEN** a client sends a payload that matches the schema and that the type's decoder refuses, to a node that
  hosts the type
- **THEN** the answer is `422 invalid-payload`, and the detail holds the words "cannot be decoded"

#### Scenario: The same payload on a node that does not host the type

- **WHEN** the same send goes to a node that does not host the type
- **THEN** the answer is `202`, and the message ends `failed` in its turn

#### Scenario: A state that the schema allows and the type cannot decode

- **WHEN** a client creates an actor with a state that matches the schema and that the type's decoder refuses, on a
  node that hosts the type
- **THEN** the answer is `422 invalid-state`, the detail holds the words "cannot be decoded", and no actor exists

### Requirement: The detail of a refusal by the schema names the schema and no place

The detail of a refusal by the schema SHALL say that the payload of the message type, or the state, does not match the
registered schema, and that `GET /types/{name}` has the schema, and SHALL be the same on every node. It SHALL name no
place of the value, quote nothing of it and hold no address of the schema. A detail SHALL be at most 2,000 characters: a
longer one SHALL be cut and SHALL end with a mark that says so.

#### Scenario: Two places that do not match

- **WHEN** a client sends a payload with two fields of the wrong kind
- **THEN** the detail names the message type and the schema and no field, on a node that hosts the type and on one
  that does not

#### Scenario: A field inside an optional part

- **WHEN** a client sends a payload whose optional field holds an object with a field of the wrong kind
- **THEN** the answer is `422 invalid-payload`, and the detail names no place

#### Scenario: A very long value

- **WHEN** a client sends a payload whose text field of 200,000 characters does not match the field's pattern
- **THEN** the answer is `422 invalid-payload`, and the detail quotes nothing of the value

#### Scenario: A very long value that only the decoder refuses

- **WHEN** a client sends a payload whose field of 200,000 characters the schema allows and the decoder refuses with a
  reason that quotes it, to a node that hosts the type
- **THEN** the answer is `422 invalid-payload`, and the detail is 2,000 characters long and ends with the mark that it
  was cut

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

### Requirement: The check uses nothing outside the schema, and asserts no format

The check SHALL NOT read a file or the network for a schema, and SHALL treat `format` as a note. A schema that the
validator cannot use SHALL NOT be enforced: the request goes on as if it matched.

#### Scenario: A date that is not a date

- **WHEN** a client sends a text that is no date in a field whose schema has the format `date-time`, to a node that
  does not host the type
- **THEN** the answer is `202`, and the message ends `failed` in its turn

#### Scenario: A schema that refers to a file

- **WHEN** the registered schema of a message type has a `$ref` to a `file:` address, and a client sends a message of
  that type
- **THEN** no file is read, and the send is accepted

#### Scenario: A schema with a pattern the validator does not know

- **WHEN** the registered schema of a message type cannot be compiled, and a client sends a message of that type
- **THEN** the send is accepted

#### Scenario: A state schema that cannot be used

- **WHEN** the registered state schema of a type cannot be compiled, and a client creates an actor of that type
- **THEN** the actor is created

### Requirement: A host can test its samples against the schema and the decoder

`cicada-axum` SHALL offer `assert_message_sample` and `assert_state_sample`. Each SHALL fail unless its sample
passes the schema of the type and decodes as the type, SHALL say which of the two refused the sample and why, for the
schema with the first place that does not match, and SHALL fail when the schema cannot be used.

#### Scenario: A sample that passes both

- **WHEN** a test gives `assert_message_sample` a message that the type serialised
- **THEN** it passes

#### Scenario: A sample that only the decoder accepts

- **WHEN** a test gives it a message that names a field by its `#[serde(alias)]`
- **THEN** it fails, and says that the schema refused the sample

#### Scenario: A sample that only the schema accepts

- **WHEN** a test gives it a message with a number that is too large for the field's type
- **THEN** it fails, and says that the decoder refused the sample

#### Scenario: A state sample

- **WHEN** a test gives `assert_state_sample` a state that the type serialised, and then one with a field of the
  wrong kind
- **THEN** the first passes and the second fails

#### Scenario: A schema that cannot be used

- **WHEN** a test gives a sample of a type whose schema the validator cannot compile
- **THEN** it fails, and says so

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
