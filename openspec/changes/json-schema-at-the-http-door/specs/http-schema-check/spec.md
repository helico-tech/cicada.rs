# http-schema-check

## ADDED Requirements

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
- **THEN** the answer is `422 invalid-payload`, and the detail names the field

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
against the schema. It SHALL get the answer of a repeated send.

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

### Requirement: A hosting node also decodes, with the same answer

After the schema check, a node that hosts the type SHALL decode the payload or the state as today, and SHALL refuse
one that does not decode with `422` and the same problem type. The detail SHALL say which of the two checks
refused it.

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

### Requirement: The detail of a refusal says where and why

The detail of a refusal by the schema SHALL name the message type, or the state, and up to five places of the
payload that do not match, each with its path in the payload and the reason, and SHALL say when there are more. It
SHALL NOT hold the address of the schema or a path into it. It SHALL be at most 2,000 characters: a longer one SHALL
be cut and SHALL end with a mark that says so. The detail of a refusal by the decoder of a hosting node SHALL be cut
in the same way.

#### Scenario: Two places that do not match

- **WHEN** a client sends a payload with two fields of the wrong kind
- **THEN** the detail names the message type and both fields, each with what was expected

#### Scenario: A field inside an optional part

- **WHEN** a client sends a payload whose optional field holds an object with a field of the wrong kind
- **THEN** the detail names the path of the inner field

#### Scenario: More than five places

- **WHEN** a client sends a payload with eight fields of the wrong kind
- **THEN** the detail names five of them and says that there are more

#### Scenario: A very long value

- **WHEN** a client sends a payload whose text field of 200,000 characters does not match the field's pattern
- **THEN** the answer is `422 invalid-payload`, and the detail is 2,000 characters long and ends with the mark that it
  was cut

#### Scenario: A very long value that only the decoder refuses

- **WHEN** a client sends a payload whose field of 200,000 characters the schema allows and the decoder refuses with a
  reason that quotes it, to a node that hosts the type
- **THEN** the answer is `422 invalid-payload`, and the detail is 2,000 characters long and ends with the mark that it
  was cut

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

### Requirement: A value at the deepest level the parser allows is checked like any other

A payload or a state as deep as the JSON parser of a request body lets through SHALL be checked against its schema like
any other value, on every node, and the check SHALL NOT end the process.

#### Scenario: A value at the deepest level the parser allows

- **WHEN** a client sends a payload of 126 levels of a type that refers to itself, in a body of 127 levels, once
  matching the schema and once with mismatches at every level, to a node that hosts the type and to one that does not
- **THEN** both nodes answer `202` for the first, which is then handled, and `422 invalid-payload` with the same detail
  for the second, and a body one level deeper is `400`

#### Scenario: A type with several schema steps between its levels

- **WHEN** a client sends a payload and creates an actor with a state of a type that has six newtypes between two of
  its levels, at 24 levels and at the deepest level the parser allows, once matching and once not, to a node that
  hosts the type and to one that does not, both served on a runtime whose threads have their default stack
- **THEN** both nodes answer `202` and `201` for the matching ones and `422` for the others, and the process goes on

### Requirement: A host can test its samples against the schema and the decoder

`cicada-axum` SHALL offer `assert_message_sample` and `assert_state_sample`. Each SHALL fail unless its sample
passes the schema of the type and decodes as the type, SHALL say which of the two refused the sample and why, and
SHALL fail when the schema cannot be used.

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
