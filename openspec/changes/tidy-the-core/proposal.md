# Proposal

## Why

Three small things make the definition API longer or vaguer than it needs to be:

- The message type is declared twice: on the ref (`ActorRef::Message`) and on the actor (`Actor::Message`), with a
  bound that makes the two equal.
- Every actor must state `const UNHANDLED`, and `Unhandled::Drop` silently drops a message that cannot be decoded.
- Many constructors return `Result<_, String>` (`MessageId::new`, `RetryPolicy::new`, `Backoff::fixed`,
  `Retention::last`, `Contract::from_json` and more), so a caller can only show the text.

ADR 0018 lists them as tidying.

## What Changes

- **BREAKING** `Actor::Message` is removed. A handler takes the message type of its ref.
- **BREAKING** `Unhandled` and `Actor::UNHANDLED` are removed. A message that cannot be decoded ends `failed`.
- **BREAKING** Constructors that returned `Result<_, String>` return a typed error.
- The detail of a `422` for a payload that cannot be decoded says its reason once. Today it repeats itself:
  `Payload of 'Step' cannot be decoded: Payload of 'Step' cannot be decoded: …` (seen on
  `POST /actors/{type}/{key}/messages`, found while `cli-and-tui` was validated).
- A send to an actor whose type is not registered says what is wrong. Today it answers `404 no-such-instance`,
  "… does not exist", while the actor exists and its type is the one that is missing (found while
  `every-actor-has-state` was validated; a formerly stateless type is in that state during an upgrade until a node
  that hosts it starts).

The simplest version: no alias and no deprecation period. An actor definition loses two lines.

To decide in the design:

- One error type per module, or one for the crate.
- Whether `StartError` gets a variant per problem. It is not a constructor and ADR 0018 does not name it, but it has
  only text today, and Chorus asks for variants (its change `cicada-requests`, point 2).

## Capabilities

### New Capabilities

- `undecodable-messages`: a message whose payload cannot be decoded ends `failed`.

### Modified Capabilities

None.

## Out of scope

- The texts and problem types of HTTP errors.
- Any change to what a handler may do.

## Order

Step 4 of ADR 0018. After `one-name-keys`, before `child-actors`.

## Impact

- `crates/cicada` (`actor`, `message`, `retry`, `retention`, `contract`, `store::model`, `api`, `runtime`), every
  actor definition in `cicada-test`, `cicada-extras`, the examples and the sample service, `README.md`,
  `docs/architecture.md` (the definition API, the load step of a turn, the term *actor type*) and `CHANGELOG.md`.
- ADR 0018 supersedes the required `UNHANDLED` of ADR 0003 and `Unhandled` in ADR 0012; no further ADR.
- Chorus and Karakuri must follow.
