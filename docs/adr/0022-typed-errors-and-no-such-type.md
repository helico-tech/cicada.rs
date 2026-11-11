# 0022. Typed errors, and "no such type" for a send

Date: 2026-11-11

## Status

Accepted

## Context

ADR 0018 lists typed errors as tidying: constructors that return `Result<_, String>` return typed errors. It names
neither which shape those errors have nor `StartError`, which holds its problems as texts. Chorus recognises a
contract conflict at start by comparing a text, and asks for something it can match on.

Three error answers were found to say the wrong thing or too much. A send to an actor whose type is not registered
answers `404 no-such-instance`, "does not exist", although the actor may exist and its type is what is missing. The
detail of a `422` for a payload that cannot be decoded repeats its reason. And a migration that violates a constraint
is reported as "store unavailable", which tells a node to try again.

The change `tidy-the-core` settles these. A changed error answer over HTTP is a change of the wire format, which the
project rule wants recorded.

## Decision

- **One named error per module, which holds its text.** A public function of the core that returned a text as its
  error returns an error type of its module: `RetryError`, `RetentionError`, `IdError`, `MessageError`,
  `ContractError`, `DefinitionError` and `RequestError`. Each holds its text, as `AddressError` does, implements
  `std::error::Error`, and prints the text it printed before. An error is an enum with a case per cause only where a
  caller matches on it. The checks of `runtime::limits` become private to the crate; they have no caller outside it.
- **A start error holds a problem per cause, of two kinds.** `StartError` holds a list of `StartProblem`, an enum
  that is open to new cases: a contract conflict, with the type name and both hashes, and any other problem, with
  its sentence. `StartError` can still be cloned, and the text it prints does not change.
- **A send to an actor of a type that is not registered answers "no such type".** `ActorSystem::send_message`
  answers `NoSuchType`, and HTTP `404 no-such-type`, as a create of such an actor does. `no-such-instance` stays for
  a registered type that has no such actor. The check for a known id still runs first: a repeated send answers as a
  duplicate while its message is pending or its outcome is still shown.
- **A retry is not a send.** A retry of a failed message is accepted whether or not the actor's type is registered;
  the message waits in the mailbox.
- **The reason of a `422` is said once.**
- **A migration that violates a constraint is not "unavailable".** When a statement of a migration violates a
  constraint of the database, the store reports an invalid store with the schema version it could not reach and the
  database's message. "Unavailable" tells a node to try again, and trying again cannot help. Every other error of a
  migration is reported as before: a lost connection, a locked database, a deadlock and a missing privilege stay
  "unavailable".

## Consequences

- Breaking for a caller that handled one of these errors as a `String`, that read `StartError.problems` as texts,
  or that matched on `SendResult` without a catch-all.
- A client that tells `no-such-instance` from `no-such-type` gets the second for a send to an unregistered type.
  Both are a `404`.
- `hash_conflict_reason` is no longer needed and leaves the public API: a host matches on the contract conflict.
- A caller cannot match on the cause inside a module error; it has the text. A module error can become an enum
  later, as a breaking change of that one type, when someone needs to match.
- `StartProblem` is a public type to keep stable. New cases can be added without a breaking change; a case that is
  removed is a breaking change.
- A node that retried its store at start stops at once on a migration that violates a constraint.
- No schema change and no contract change: nodes before and after can share a database.
