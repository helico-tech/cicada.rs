# Proposal

## Why

Three small things make the definition API longer or vaguer than it needs to be:

- The message type is declared twice: on the ref (`ActorRef::Message`) and on the actor (`Actor::Message`), with a
  bound that makes the two equal.
- Every actor must state `const UNHANDLED`, and `Unhandled::Drop` silently drops a message that cannot be decoded.
- Many constructors return `Result<_, String>` (`MessageId::new`, `RetryPolicy::new`, `Backoff::fixed`,
  `Keep::last`, `Contract::from_json` and more). A `String` is not an error type: `?` does not carry it into
  `anyhow` or into an error of the host, and a signature does not say what failed.

ADR 0018 lists them as tidying.

Five more things came in on the way. Each is an error that says the wrong thing or too little:

- `StartError` holds its problems as texts, so Chorus recognises a contract conflict by comparing a text (its change
  `cicada-requests`, point 2).
- The detail of a `422` for a payload that cannot be decoded repeats its reason (found while `cli-and-tui` was
  validated).
- A send to an actor whose type is not registered answers "does not exist", although the actor may exist (found
  while `every-actor-has-state` was validated).
- A migration that violates a constraint is reported as "store unavailable", which tells a node to try again (found
  while `one-name-keys` was validated).
- A ref whose `Serialize` writes a unit while its `Deserialize` reads a string gets the address of a singleton
  without an error (found while `one-name-keys` was validated).

## What Changes

- **BREAKING** `Actor::Message` is removed. A handler takes the message type of its ref.
- **BREAKING** `Unhandled` and `Actor::UNHANDLED` are removed. A message that cannot be decoded ends `failed`.
- **BREAKING** Functions that returned a text as their error return a named error type of their module. It holds
  the text, prints it as before and implements `std::error::Error`: `RetryError`, `RetentionError`, `IdError`,
  `MessageError`, `ContractError`, `DefinitionError` and `RequestError`.
- **BREAKING** The checks `runtime::limits::check`, `check_envelope` and `check_json` are no longer public; nothing
  outside the core calls them. The limits themselves stay public.
- **BREAKING** `StartError` holds a problem per cause, of two kinds: a contract conflict with the type name and
  both hashes, and any other problem with its sentence. It can still be cloned and prints what it printed before.
  `hash_conflict_reason` is no longer public.
- The detail of a `422` for a payload that cannot be decoded says its reason once. Today it repeats itself:
  `Payload of 'Step' cannot be decoded: Payload of 'Step' cannot be decoded: …` (seen on
  `POST /actors/{type}/{key}/messages`, found while `cli-and-tui` was validated).
- **BREAKING** A send to an actor whose type is not registered answers `404 no-such-type`, as a create does. Today
  it answers `404 no-such-instance`, "… does not exist", while the actor exists and its type is the one that is
  missing (found while `every-actor-has-state` was validated). The check for a known id still runs first, so a
  repeated send answers as a duplicate as today. A retry of a failed message is not such a send: it is accepted as
  today.
- When a statement of a migration violates a constraint, the store reports `StoreError::Invalid` with the reason,
  not `Unavailable`, so that a node stops and does not retry. Every other error of a migration is reported as today:
  a lost connection, a locked database, a deadlock and a missing privilege stay `Unavailable`.
- A ref that writes a unit and does not read as one has no address: a send to it or a create of it fails at the
  call.

The simplest version: no alias and no deprecation period. An actor definition loses two lines.

## Capabilities

### New Capabilities

- `undecodable-messages`: a message that cannot be decoded ends `failed`, and a refusal at the door says why once.
- `typed-errors`: a function of the core returns a named error of its module, a start error holds a problem per
  cause, and a migration that violates a constraint is not reported as "unavailable".

### Modified Capabilities

- `type-registry`: a send to an actor of a type that is not registered answers "no such type"; a retry of a failed
  message does not need the type.
- `actor-addressing`: a ref that writes a unit must also read as one.

## Out of scope

- Other texts and problem types of HTTP errors than the two named above.
- Turning the error types that already exist and wrap a text (`AddressError`, `TimeError`) into enums.
- The text errors of `cicada-extras` and `cicada-test`, and the text in `TurnError::Invalid`.
- Any change to what a handler may do.

## Order

Step 4 of ADR 0018. After `one-name-keys`, before `child-actors`.

## Impact

- `crates/cicada` (`actor`, `address`, `message`, `schema`, `retry`, `retention`, `contract`, `store` with both
  backends, `api`, `runtime`, `turn`, the docs of `plugin`), `cicada-axum` (it builds the detail of the `422` and
  maps the new answer), `cicada-client` and `cicada-cli` (callers of the constructors, and the new answer of a
  send), every actor
  definition in `cicada-test`, `cicada-extras`, `cicada-otel`, the examples, the sample service and the tests of
  `cicada-axum`, `cicada-client` and `cicada-cli`, `README.md` (the definition API, the HTTP table and the table of
  ADRs), `docs/architecture.md` (the definition API, the load step of a turn, the term *actor type*),
  `docs/operations.md` (what a node does when a migration fails) and `CHANGELOG.md`.
- ADR 0018 supersedes the required `UNHANDLED` of ADR 0003 and `Unhandled` in ADR 0012. ADR 0022 records the typed
  errors, the changed answer of a send and the error of a migration that violates a constraint.
- Chorus and Karakuri must follow.
