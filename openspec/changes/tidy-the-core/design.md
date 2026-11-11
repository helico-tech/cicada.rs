# Design

Described with the owner on 2026-11-11 (change loop, Describe). Every decision below is made by the owner unless it
says otherwise. Reviewed three times. Each review made the design smaller: the module errors became wrappers, the
start problems went from seven cases to two, and the rule for a failed migration was narrowed to a violated
constraint. After the third review the owner left the open points to the main session (2026-11-11: "you have my
approval to decide for me"); what it decided then is marked "third review".

`one-name-keys` has landed (0.6.0); the lists below are counted on that code.

## Context

What the code does today (`main` at 51eb908, release 0.6.0):

- `Actor` declares the message type twice: `type Ref: ActorRef<Message = Self::Message>` and
  `type Message: MessageFamily`. A handler is `async fn handle(&self, turn: &mut Turn<Self>, message: Self::Message)`.
  `Turn::send_self` and `send_self_with` name `A::Message`.
- Every actor states `const UNHANDLED: Unhandled`. Seven definitions in this repository say `Drop`: two in the tests
  of `cicada-axum`, one in the tests of `cicada-client`, three in the tests of the core, and
  `examples/src/singleton.rs`. All others say `Fail`. With `Fail`, a message that cannot be decoded ends `failed` with
  the reason, without a retry, and no handler runs. With `Drop` it ends `done`.
- The names of a message family are checked before serde decodes a payload (`actor.rs`, `load`). So a message with a
  type name the family does not have never reaches a variant, whatever the family declares.
- These public functions of the core return a text, or a list of texts, as their error:

  | Module | Functions |
  | --- | --- |
  | `retry` | `RetryPolicy::new`, `Backoff::exponential`, `Backoff::fixed`, and the `TryFrom` behind their serde forms |
  | `retention` | `Keep::keep_for`, `Keep::last`, and the `TryFrom` behind their serde form |
  | `store` | `MessageId::new`, `NodeId::new`, and their `TryFrom<String>` (one macro) |
  | `message` | `encode`, `decode`, `FamilyInfo::of` |
  | `schema` | `message_schemas` |
  | `contract` | `Contract::from_json`, `Contract::of` |
  | `actor` | `contract_of`, which returns a list of texts |
  | `runtime::limits` | `check`, `check_envelope`, `check_json`; nothing outside the core calls them |
  | `api` | `BulkRequest::selection`, the `delivery` of a send request |

- Error types that exist: `AddressError` and `TimeError` wrap a text; `TurnError`, `OpError` and `StoreError` are
  enums; `StartError` holds `problems: Vec<String>` and can be cloned. `StoreError` cannot.
- A start can report these problems, each as a sentence:
  - a setting of the configuration that is out of range (six settings);
  - a plugin that is installed twice;
  - a malformed actor definition: its ref, its messages or its `MAX_TURN`, one sentence per reason. Each begins with
    the Rust type name of the actor;
  - a type that is hosted twice, a contract that differs from the registered one, and a store failure while
    registering. These three sentences also begin with the Rust type name of the actor;
  - a node that is started when it is not new: "Actor system is STARTED".
- `message::decode` writes "Payload of 'T' cannot be decoded: …". `cicada-axum` (`handlers.rs`) puts the same words
  in front again when it builds the detail of a `422`.
- `ActorSystem::send_message` first answers `Duplicate` for an id that was accepted before. Then it answers
  `NoSuchInstance` when the type of the target is not registered, which HTTP shows as `404 no-such-instance`, "… does
  not exist". A create of such an actor answers `404 no-such-type`. A retry of a failed message
  (`POST /messages/{id}/retry`) is accepted whether or not the type is registered.
- Chorus recognises a contract conflict at start by comparing a text with `cicada::store::hash_conflict_reason` and
  asks for variants on `StartError` (its change `cicada-requests`, point 2). That function is public for this reason
  only.
- A constraint that a migration violates is reported as `StoreError::Unavailable`, on both stores, like a lost
  connection. When migration 3 of 0.6.0 meets two keys that become one name, an operator reads "store unavailable",
  and a node that retries its store keeps retrying. PostgreSQL already reports data it cannot hold (SQLSTATE classes
  `22` and `54`) as `Invalid`, on every call.
- A PostgreSQL node whose user may not create tables can start before the node that migrates. It gets "permission
  denied", which is `Unavailable`; a node that retries its store gets through once the schema is migrated.
- A plugin reads why a message could not be decoded from `TurnInfo::unhandled()`. The docs of `plugin.rs` and of
  `TurnOutcome::Done` speak of the unhandled policy.
- A ref is a singleton when it declares no field to serde. The check when a ref is used asks what its `Serialize`
  writes; a ref that writes a unit is taken as a singleton without asking whether its `Deserialize` agrees. So a ref
  whose two serde halves contradict each other, writing a unit and reading a string, starts a node and gets the
  address `type/_` for every value (`address/reader.rs`, `Writer::unit`).
- Outside the core, `CrashLoopGuard::new` and `DeadLetters::with_capacity` (`cicada-extras`) and `check_contracts`
  and `check_state_decodes` (`cicada-test`) return a text as their error.

## Goals / Non-Goals

Goals:

- An actor definition says its message type once and has no unhandled policy.
- A function of the core returns a named error, not a text.
- A host can tell a contract conflict from what stopped a start, without reading a text.
- Four errors say what is wrong: a send to a type that is not registered, a payload that is refused, a migration
  that violates a constraint, and a ref whose two serde halves disagree.

Non-Goals:

- New behaviour for a handler, or a way to keep `Drop`.
- A case per rule on an error that nobody matches on.
- The text errors of `cicada-extras` and `cicada-test`, and `TurnError::Invalid`, which keeps its text.
- Turning the error types that already exist and wrap a text (`AddressError`, `TimeError`) into enums.
- Other texts and problem types of HTTP errors.

## Decisions

### 1. One named error per module, which holds its text

Each module whose public functions returned a text gets an error type of its own. It holds its text, as
`AddressError` does today, and prints the text that is printed today, so what a user reads does not change.

| Module | Error | For |
| --- | --- | --- |
| `retry` | `RetryError` | `RetryPolicy::new`, `Backoff::exponential`, `Backoff::fixed`, their `TryFrom` |
| `retention` | `RetentionError` | `Keep::keep_for`, `Keep::last`, their `TryFrom` |
| `store` | `IdError` | `MessageId::new`, `NodeId::new`, their `TryFrom<String>` |
| `message` | `MessageError` | `encode`, `decode`, `FamilyInfo::of`, `schema::message_schemas` |
| `contract` | `ContractError` | `Contract::from_json`, `Contract::of` |
| `actor` | `DefinitionError` | `contract_of` |
| `api` | `RequestError` | `BulkRequest::selection`, the `delivery` of a send request |

`DefinitionError` holds the list of reasons that `contract_of` returns today, and prints them joined with "; ", as
the core joins them today.

Each of the seven implements `std::error::Error`. That is what a text lacks: `?` carries the error into `anyhow` or
into an error type of the host, and it can be the source of another error.

An enum with a case per rule exists only where someone matches on it. Nothing in this repository, and nothing we
know of in a host, matches on these seven. A rule that is added or removed later does not break a wrapper.

The three checks of `runtime::limits` (`check`, `check_envelope`, `check_json`) become private to the crate (third
review, main session). Nothing outside the core calls them, so they need no error type. The limits themselves, the
constants of that module, stay public.

First the owner chose an enum for every module, then enums for four and wrappers for four. Each review showed fewer
callers that match, and the owner ended at wrappers for all.

Not chosen: one error enum for the whole crate (a signature would no longer say which errors a function can
return, and the enum would grow with every module).

### 2. `StartError` holds a problem per cause, of two kinds

`StartError.problems` becomes a list of `StartProblem`, an enum that is open to new cases (`#[non_exhaustive]`). It
has the case that someone matches on, and one for the rest:

| Case | It holds |
| --- | --- |
| a contract conflict | the Rust type name of the actor, the type name, the registered hash, the hash of this node, and the reason |
| any other problem | its sentence, as printed today |

- Chorus matches on the contract conflict. `hash_conflict_reason` becomes private again.
- A setting out of range, a plugin installed twice, a malformed definition (one problem per reason, as the lines are
  today), a type hosted twice, a store failure while registering and a node that is not new are "any other problem".
  A case of its own can be added for one of them later, without a breaking change.
- Each case prints the sentence that is printed today, so the text of a `StartError` does not change, and
  `StartError` stays clonable.

First the design listed seven cases. The second review showed that only one has a known matcher, and that
`turn-observer` removes the plugin case again, which would then be a breaking change; the owner chose three: the
conflict, a store failure that holds the store's error, and the rest. The third review showed that the store failure
has no known matcher either, and that it puts a shared pointer to `StoreError` into the public API. The main session
dropped it (third review): the rule of decision 1 holds here too, and a store that cannot be reached is seen
earlier, when the store is opened, as a `StoreError`.

Not chosen: text only (Chorus would keep comparing texts); a case per cause (cases that nobody matches on).

### 3. A send to an actor of a type that is not registered answers "no such type"

`SendResult` gets `NoSuchType`, and `ActorSystem::send_message` answers it when the type of the target is not
registered. Over HTTP that is `404 no-such-type`, as a create of such an actor answers today. `NoSuchInstance` stays
for a registered type that has no such actor.

The check for a known id still runs first, as today. So a send with an id that was accepted before answers
`Duplicate` while that message is pending or its outcome is still shown, whatever became of the type: that is what
makes a send safe to repeat. That check does not see an outcome that its retention policy hides; a send with such
an id to a type that is not registered answers `NoSuchType`. The order of the checks does not change (third review,
main session).

Not chosen: keeping `no-such-instance` and correcting only the text.

### 4. What follows, with no choice to make

Decided by the main session; it follows from the proposal and ADR 0018.

- **The message type is declared once.** `Actor::Message` goes. The trait writes a handler's message as the message
  type of its ref, and an implementation names that type as it does today. `send_self` takes the same type. Generic
  code that named `A::Message` names `<A::Ref as ActorRef>::Message`.
- **`Unhandled` and `Actor::UNHANDLED` go.** A message that cannot be decoded ends `failed` with the reason, without
  a retry, and no handler runs: what `Fail` does today. The seven definitions that say `Drop` lose the line; their
  tests of a dropped message go. A type cannot opt out: a message with a type name the family does not have never
  reaches a variant.
- **`TurnInfo::unhandled()` stays**, with its name: it tells a plugin why a message could not be decoded, when no
  handler runs for it. Its doc comment, the table at the top of `plugin.rs` and the doc of `TurnOutcome::Done` stop
  naming a policy (third review).
- **The reason of a `422` is said once.** `message::decode` returns a `MessageError`, and `cicada-axum` stops
  putting its words in front again.
- **The scope line of the proposal changes** to "other texts and problem types of HTTP errors".
- **No schema change and no contract change.** A contract does not hold the unhandled policy, and the message
  schemas stay as they are, so no hash changes.

### 5. A retry is not a send

A retry of a failed message puts it back into the mailbox of its actor. It is accepted whether or not the actor's
type is registered, as today: the message waits until a node that hosts the type starts, like every message that is
already in a mailbox. Decision 3 is about a send from outside a turn only.

Not chosen: refusing a retry to an actor of an unregistered type with `404 no-such-type` as well.

### 6. A short ADR records the typed errors and the changed answers

ADR 0018 lists typed errors as tidying. It does not name `StartError`, and it does not say that an error answer
changes. `docs/adr/0022-typed-errors-and-no-such-type.md` records decisions 1, 2, 3, 5 and 8 and the `422`. The main
session wrote it with this design and commits it with the change when it is ready. The HTTP table and the table of
ADRs in the README get their rows; `docs/architecture.md` has no HTTP table.

Not chosen: no ADR.

### 7. The additions are named in the Why

Decided by the main session after the first review.

The proposal's Why named three things, all from ADR 0018. Five more came in on the way and are named there with
where they came from: the kinds of `StartError` (asked for by Chorus), the reason of a `422` said once (found while
`cli-and-tui` was validated), the answer for a type that is not registered (found while `every-actor-has-state` was
validated), and decisions 8 and 9 (found while `one-name-keys` was validated). They stay in this change because each
is an error that says the wrong thing or is missing, which is what this change is about. The owner confirmed that
for decisions 8 and 9.

### 8. A migration that violates a constraint says so

When a statement of a migration violates a constraint of the database, the store fails the open with
`StoreError::Invalid`. The error names the schema version it could not reach and holds the database's own message.
That is the case of two keys that become one name in migration 3.

Everything else during a migration is reported as it is today. A connection that is lost, a SQLite that is busy and
a PostgreSQL deadlock or serialization failure stay `Unavailable`: for those, trying again is right. For a violated
constraint it cannot help, and "unavailable" tells a node to try again. A node that retries its store at start
therefore stops at once on this error. Both stores follow the same rule.

What counts, decided by the main session (third review). The rule adds one answer to what the stores report as
invalid today, and takes none away:

| Store | A violated constraint is |
| --- | --- |
| PostgreSQL | SQLSTATE class `23` |
| SQLite | result code `SQLITE_CONSTRAINT` (19), with its extended codes |

- The rule covers the statements of the migrations themselves, the lists in the two schema modules. The statements
  around them (creating the schema and the version table, writing the version) keep their mapping, and so does
  every other call of a store.
- "Permission denied" (`42501`) stays `Unavailable`. A PostgreSQL node whose user may not create tables, started
  before the node that migrates, keeps retrying and gets through, as today.
- A wrong statement in a migration (class `42`, `SQLITE_ERROR`) would be a bug of ours that the upgrade tests catch;
  it gets no rule.

The owner chose "only when the database refuses a statement". The second design listed three PostgreSQL classes and
two SQLite codes for that. The third review showed that class `42` would stop the node without privileges, which
today gets through. The list is now the one case we know.

Not chosen: `Invalid` for every error during a migration (a fault that passes would stop a node); `Invalid` for a
missing privilege as well (it would break a set-up that works today).

### 9. A ref that writes a unit must also read as one

When a ref is used and its `Serialize` writes a unit, the reader asks, as it does for a struct without fields,
whether the ref declares no field to serde. A ref that writes a unit and reads a string has no address, so a send to
it or a create of it fails at the call. One line in `Writer::unit`, and a test with the three ways the validator of
`one-name-keys` found: a `Serialize` written by hand, `into` a unit type with `try_from = "String"`, and
`serialize_with` on a newtype.

## Risks / Trade-offs

- **A host that relied on `Drop` now gets `failed` outcomes** for messages it cannot decode, and later a
  notification for each when it is watched. → The upgrade note says so, and that there is no way to opt out.
- **Nodes of 0.6.0 and 0.7.0 on one database end the same undecodable message differently** for a type that said
  `Drop`: `done` on the old node, `failed` on the new one. → The upgrade note says to upgrade every node that hosts
  such a type together.
- **A wrapper tells a caller nothing but its text.** → That is what the callers use today. A case per rule can
  replace a wrapper when someone needs to match, as a breaking change of that one type.
- **A client that matches on `no-such-instance` for a send to an unregistered type** gets `no-such-type` now. →
  Both are a `404`; the upgrade note names the case.
- **A node that used to keep retrying on a migration that violates a constraint now stops.** → That is the point;
  the operator sees the reason at once.
- **`runtime::limits::check`, `check_envelope` and `check_json` leave the public API.** → Nothing in this repository
  calls them from outside the core; the changelog names them.

## Migration Plan

For a host (Chorus, Karakuri):

1. In every actor definition, remove `type Message = …;` and `const UNHANDLED: Unhandled = …;`. Generic code that
   named `A::Message` names `<A::Ref as ActorRef>::Message`.
2. A type that said `Unhandled::Drop`: expect `failed` for a message that cannot be decoded. Upgrade every node that
   hosts such a type together.
3. Where a function's error was handled as a `String`, print the error, or keep it as the named type of its module.
   `runtime::limits::check`, `check_envelope` and `check_json` are no longer public.
4. Match on `StartProblem` instead of comparing texts; `hash_conflict_reason` is gone.
5. A caller of `send_message` handles `SendResult::NoSuchType`.
6. A node that retried its store at start stops on a migration that violates a constraint; read the reason.
7. A ref whose `Serialize` writes a unit while its `Deserialize` reads a string has no address any more; make the
   two agree.

The text errors of `cicada-extras` (`CrashLoopGuard::new`, `DeadLetters::with_capacity`) and of `cicada-test`
(`check_contracts`, `check_state_decodes`) stay as they are.

No database changes, and no contract hash changes. The version is a breaking one for the API: 0.7.0 after 0.6.0.
