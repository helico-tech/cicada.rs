# type-registry

## ADDED Requirements

### Requirement: A message schema holds every definition it refers to

The schema of each message type of a contract SHALL hold every definition that it refers to, also one whose name is
escaped in a reference: a name with `~`, `/`, a space, `<`, `>`, another character outside the safe set of a
fragment, or a letter that is not ASCII, whether the name comes from `#[schemars(rename)]`, from a container
`#[serde(rename)]` or from the Rust type. A reference of several segments SHALL need the definition of its first
segment.

#### Scenario: A renamed type

- **WHEN** a message holds a field of a type that is renamed to `Line <v2>` with `#[schemars(rename)]`
- **THEN** the schema of that message type holds the definition `Line <v2>`, and the door checks a send of it

#### Scenario: A type renamed for serde

- **WHEN** a message holds a field of a type with the container attribute `#[serde(rename = "line/v2")]`
- **THEN** the schema of that message type holds the definition `line/v2`

#### Scenario: A type name with a letter that is not ASCII

- **WHEN** a message holds a field of the type `Größe`
- **THEN** the schema of that message type holds the definition `Größe`

#### Scenario: A newtype variant of a renamed type

- **WHEN** a variant of a message family is a newtype of a renamed type with a space in its name
- **THEN** the node starts, and the schema of that message type is the schema of that type, with the definitions it
  refers to

#### Scenario: A newtype variant whose definition is missing

- **WHEN** a hand-written family schema has a newtype variant that refers to a definition the schema does not hold
- **THEN** the error names the variant and the missing definition

#### Scenario: A reference of several segments

- **WHEN** a hand-written message schema refers to `#/$defs/Line/properties/sku`
- **THEN** the schema of that message type holds the definition `Line`

#### Scenario: A type that is not hit

- **WHEN** the contract of a type whose definition names need no escaping, whose family does not hold itself, and
  whose references have one segment, is made
- **THEN** it is the same as before, byte for byte

### Requirement: A newtype variant refers to a whole definition

A newtype variant of a message family SHALL refer to a whole definition, so that the door is never stricter than the
decoder. A family with a newtype variant whose reference points inside a definition SHALL be refused, with an error
that names the variant and the reference. A reference of one segment SHALL NOT be affected.

#### Scenario: A newtype variant that points inside a definition

- **WHEN** a hand-written family schema has a newtype variant that refers to `#/$defs/Line/properties/sku`
- **THEN** the error names the variant and the reference, and no schema of the family is made

### Requirement: A family that holds itself is never refused for its nested messages

When a message family holds itself, a reference to the whole family in the schema of one message type SHALL accept
any value, so that the door never refuses a valid nested message; the decoder checks it. The schema of a message
type that holds no such reference SHALL NOT change.

#### Scenario: A batch of messages

- **WHEN** a family has a message `Batch { items: Vec<Msg> }` and a client sends a `Batch` whose items are an `Add`
  and a `Batch`
- **THEN** the door accepts it on every node, as the type decodes it

#### Scenario: A nested message that is wrong, on a node that hosts the type

- **WHEN** an item of such a `Batch` has a field of the wrong kind, and the send goes to a node that hosts the type
- **THEN** the answer is `422`, from the decoder

#### Scenario: A nested message that is wrong, on a node that does not host the type

- **WHEN** the same send goes to a node that does not host the type
- **THEN** it is accepted, and the message ends `failed` with the cause `message` in its turn

#### Scenario: The other messages of the family

- **WHEN** a family has `Batch` and `Add`, and only `Batch` refers to the family
- **THEN** the schema of `Add` is the same as before, byte for byte

#### Scenario: A `#` that is data

- **WHEN** a hand-written schema of a message holds `"#"` as a `const` value
- **THEN** that value stays as it is
