## ADDED Requirements

### Requirement: A host registers a contract without hosting its implementation

ActorSystem::register_contract SHALL accept a Contract without starting or hosting its actor. It SHALL compute the
hash from the name and schemas, then use the store's atomic registration. An equal hash SHALL return the stored
contract; a different hash SHALL return a conflict without replacing it.

#### Scenario: Registration on a system without handlers

- **WHEN** a host registers a generated counter contract on an unstarted ActorSystem with no hosted types
- **THEN** registration succeeds, type reads show it, and no handler, actor, node row or actor-change hint is created
- **AND** hosted_types stays empty and queued counter messages are not executed by that system

#### Scenario: A hash supplied through a registry definition

- **WHEN** a Contract created with Contract::of carries a supplied hash that differs from its schemas' computed hash
- **AND** it is passed to register_contract
- **THEN** the operation stores and returns the computed hash, never the supplied hash

#### Scenario: An equal registration preserves the stored descriptions

- **WHEN** a second registration changes only schema descriptions
- **THEN** it succeeds with the first stored contract and hash, retaining the first descriptions

#### Scenario: A conflicting registration

- **WHEN** a registered type receives a contract with a different computed hash
- **THEN** the result identifies the existing registration and requested hash and leaves the registration unchanged
- **AND** this holds whether or not an actor of the type exists

#### Scenario: A peer replaces descriptions after deletion

- **WHEN** one ActorSystem has cached description A, another deletes the unused registration and registers description B
- **AND** both contracts have the same description-insensitive hash
- **THEN** subsequent registration results and type reads through either system use stored description B
- **AND** neither Contract hash equality nor a stale local cache may substitute description A

#### Scenario: Concurrent first registrations

- **WHEN** clients concurrently register two different contracts for the same absent type
- **THEN** exactly one contract becomes registered and requests for the other return conflict
- **AND** concurrent registrations of an equal hash all succeed with the stored contract

#### Scenario: A local host starts later

- **WHEN** a node hosting the same type initializes after registration through the new operation
- **THEN** its equal generated contract succeeds and a different generated contract fails with the existing startup
  contract-conflict result

### Requirement: New contract registration checks structure without compiling schemas

The new registration operation SHALL require object or boolean schema roots, send-compatible message names and
NUL-free schema strings and keys. Invalid contracts SHALL leave the registry unchanged. Registration SHALL NOT
compile schemas, retrieve references or apply the 64-level actor-value depth limit to schema documents.

#### Scenario: Basic schema shapes

- **WHEN** the state schema or a message schema is null, a number, a string or an array
- **THEN** the operation returns InvalidContract without a store write
- **AND** object and boolean roots and an empty messages map are accepted

#### Scenario: Message names follow send rules

- **WHEN** a message name exceeds 200 characters or contains a NUL character
- **THEN** registration returns InvalidContract without a store write
- **AND** names within the existing send-name rules, including empty names, are not rejected by actor-type grammar

#### Scenario: A schema contains a NUL

- **WHEN** any string or object key inside a schema contains a NUL character
- **THEN** registration returns InvalidContract before hashing or writing, on every backend

#### Scenario: A schema the HTTP validator cannot compile

- **WHEN** a structurally acceptable contract includes an unresolved external reference or unsupported schema feature
- **THEN** registration stores it without retrieving or compiling it
- **AND** later HTTP creates and sends retain the existing unusable-schema behavior and handler decode checks

### Requirement: Newly registered schemas remain readable through storage and HTTP

The new registration operation SHALL limit each complete state or message schema to 120 nested arrays/objects,
with a root container counting as one and a scalar as zero. A deeper schema SHALL return InvalidContract before
writing. The stored contract and TypeView/TypeListView wrappers SHALL fit the existing 128-level JSON parsers.

#### Scenario: The accepted schema depth boundary

- **WHEN** a client registers state and message schemas each 120 levels deep through the new operation
- **THEN** registration, SQL store reopen and read, equal registration retry, and HTTP client decoding of the type
  and the type list all succeed
- **AND** this is checked on SQLite and PostgreSQL as well as memory

#### Scenario: A schema beyond the bound

- **WHEN** a state or message schema is 121 levels deep
- **THEN** the core returns InvalidContract and HTTP returns 422 invalid-contract, leaving the registry unchanged
- **AND** this also holds when excessive nesting appears in schema data such as defaults or examples

#### Scenario: Schema and value bounds are separate

- **WHEN** a schema is more than 64 but at most 120 levels deep and otherwise valid for registration
- **THEN** it is not refused by the actor-value depth limit
- **AND** HTTP retains its existing parser and body-size limits, and local initialization keeps its existing rules

### Requirement: A client registers a type over HTTP

PUT /types/{name} on the full router SHALL accept required state and messages fields and no other envelope fields.
The server SHALL derive the type name from the path, compute the hash and answer 200 with the stored TypeView for a
new or equal registration. The read-only router SHALL NOT serve the write operation.

#### Scenario: A registration request

- **WHEN** a client puts an object containing the state schema and message-schema map to /types/counter
- **THEN** the response is 200 with name, hash, state and messages equal to the accepted stored TypeView
- **AND** an identical retry succeeds without changing the registration

#### Scenario: A malformed request

- **WHEN** the body is empty, malformed JSON, not an object, missing state or messages, or has non-object messages
- **THEN** the answer is 400 bad-request and nothing is registered

#### Scenario: Client-supplied identity fields

- **WHEN** the body includes name, type, hash or any other unknown envelope field
- **THEN** the answer is 400 bad-request rather than ignoring or trusting the field
- **AND** unknown keywords within a schema document are preserved

#### Scenario: An invalid path or contract

- **WHEN** the path name is not a valid TypeName
- **THEN** the answer is 400 bad-request
- **WHEN** a parsed request has an invalid contract under the new registration rules
- **THEN** the answer is 422 invalid-contract and nothing is registered

#### Scenario: A wire conflict

- **WHEN** a client registers a different contract for an existing name
- **THEN** the answer is 409 contract-conflict in the ordinary Problem shape
- **AND** its detail identifies the type and both hashes, while the existing registration stays unchanged

#### Scenario: Existing body and store errors

- **WHEN** a request exceeds the configured HTTP body limit
- **THEN** it receives 413 request-too-large without a registration
- **WHEN** a valid registration request reaches a closed store
- **THEN** it receives 503 store-unavailable

#### Scenario: Read-only mounting

- **WHEN** a client sends PUT /types/counter to actor_api_read
- **THEN** it receives the existing 405 method-not-allowed response and no registration occurs

#### Scenario: The registered schemas are used before a handler exists

- **WHEN** a contract is registered over HTTP on a system with no handlers
- **THEN** a create and send matching its usable schemas succeed and the message waits for a hosting worker
- **AND** a create or send that violates those usable schemas is refused by the existing schema gate

### Requirement: The typed client and OpenAPI describe registration

ActorClient::register_type SHALL send a Contract's state and messages without a hash and return the stored TypeView.
It SHALL preserve ordinary problem errors on refusal. OpenAPI SHALL describe registerType with its request,
200 TypeView and problem responses only on the full router.

#### Scenario: Typed registration

- **WHEN** the client registers a generated Rust actor contract
- **THEN** the stored hash equals the locally generated hash and registration does not require a server handler
- **AND** a conflicting contract returns the client's normal Problem error without parsing its detail text

#### Scenario: Generated API description

- **WHEN** the OpenAPI documents are generated
- **THEN** the full document includes PUT /types/{name} as registerType, the read document has no PUT on that path,
  and docs/openapi.json matches the generated full document
