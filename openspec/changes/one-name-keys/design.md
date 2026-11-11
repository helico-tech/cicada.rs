# Design

Described with the owner on 2026-11-11 (change loop, Describe), while `private-store-trait` is being built. Every
decision below is made by the owner unless it says otherwise. Reviewed twice. After the first review the rule of
decision 1 is stated as serde sees it, decision 4 names the whole effect of its window and rewrites less, and
decision 6 was added. After the second the shapes of decision 1 are listed, the specs name the schema versions they
are about, and the upgrade steps say what happens to a ref that changes its shape.

`private-store-trait` lands first and moves the stores into `cicada`, so "both stores" below means their modules
there.

## Context

What the code does today (`main` at 134b4b4, after release 0.4.0):

- A ref is a serde struct. Its named fields form the key: one value, or several joined with `:` in declared order.
  A unit struct is a singleton with the key `_` (ADR 0013).
- `crates/cicada/src/address/params.rs` (492 lines) is a serde bridge that writes each field of a ref as text and
  parses it back by field type: strings, booleans, integers, floats, chars, unit enum variants and newtypes.
- Each value is percent-encoded (`address/percent.rs`, 92 lines): `A-Z a-z 0-9 . _ ~ -` stay as they are, every
  other character becomes `%XX`. So a raw `:` is always a separator, and a literal colon in a value is `%3A`. The
  keys `.` and `..` are refused.
- `ActorAddress::parse` writes every address in its canonical spelling: it decodes each value and encodes it again,
  so `sync/%61cme` becomes `sync/acme`.
- A node that hosts a type refuses an address that is not canonical for its ref, such as `x/007` for an integer
  field (`canonical_problem`).
- A contract holds the names of the key's fields, and they count in its hash. `TypeView.key` and `ActorView.params`
  show them over HTTP; the CLI has columns for them. 19 places outside the address module use them.
- Contract snapshots are compared by a hash that is computed again from the file
  (`crates/cicada-test/src/snapshots.rs`).
- The refs in this repository: 56 have one field, 9 have none, 4 have two. Of the 56, two hold a number
  (`Num { n: i32 }` and `Worker { id: i32 }`, in tests of `cicada-test`); the others hold a `String`. The refs with
  two fields have a number second (`BatchItem { batch, index: i32 }`, `Account { tenant, id: i32 }`).
- Chorus wraps every ref in one generic type of its own, `Named<A>`: a `name: String` and a `PhantomData` that serde
  skips. It calls `ActorAddress::of` twice. Karakuri has no cicada dependency.
- The migration of 0.4.0 removed the registrations of the formerly stateless types, so that a database upgrades in
  place. While a type is not registered: its actors can be read and deleted and messages already in a mailbox wait;
  a create of one and a send to one from outside a turn are refused; a turn that creates one ends `failed` without
  a retry (`check_typed_effects`), and a raw send in a turn is refused.

## Goals / Non-Goals

Goals:

- A key is one name. The core no longer knows fields of a key, their names or their types.
- A ref that serde writes as one string, and every actor that exists, keeps working without a change.
- A contract holds no key fields.
- A database of schema version 2 is upgraded in place.

Non-Goals:

- Namespaces or tenancy across types; the rules for type names; the paths of the HTTP API.
- Typed parts of a key: a caller that wants a number in its key formats and parses it.
- A helper that joins or splits the parts of a name.

## Decisions

### 1. A ref is written as one string, or as a unit

A ref keeps its shape. What counts is what serde does with it:

- a struct of which serde writes one field, whose value is a string. A field that serde skips does not count, so
  Chorus's `Named<A>` with its `PhantomData` is a ref. The field may be a `String` or a type that serde writes as a
  string, such as a newtype around one or a `Uuid`;
- a type that serde writes as a string itself: a newtype around a `String`, or a type that names `String` as its
  serde form;
- a unit struct, for a singleton with the key `_`.

Only a string counts. A number, a boolean, a single character and an enum variant are refused, although the last
two work as a key field today: they are typed parts of a key, and a caller formats those itself.

The refs of this repository with one `String` field or none do not change. A small reader replaces the serde bridge
of `params.rs`: it takes the one string out of a ref and puts it back.

A ref with two parts joins them itself. It can hold one string and build it in a constructor, or keep its typed
fields and tell serde how to write itself as one string:

```rust
#[derive(Clone, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct BatchItem { pub batch: String, pub index: i32 }   // written as "b1.7"
```

A ref that is not one string is a mistake that the compiler does not see. It is caught twice:

- **When a node starts**, by what the ref's `Deserialize` asks for: no value exists yet. A node that hosts a type
  whose ref asks for a second field, or for something that is not a string, reports it and does not start, as for a
  ref that is not a struct today.
- **When a ref is used**, by what its `Serialize` writes. A ref that writes anything but one string has no address,
  so a send to it or a create of it fails at the call. This also catches a ref of a type the node does not host,
  and a ref whose two serde halves disagree.

Not chosen: accepting a single character and an enum variant as well (they are text in JSON and work today, but
they bring typed parts of a key back); two functions on the trait, `key()` and `from_key()` (every ref gets two
more functions, here and in every host); one generic type `Ref<T>` that holds a name (named fields go, and every
place that builds a ref changes).

### 2. A name is any text, percent-encoded as today

A name may hold any text and must not be empty. In an address it is percent-encoded as today, so `jan@example.com`
shows as `jan%40example.com`. The names `.` and `..` stay refused. Nothing changes for a key that works today.

Not chosen: a fixed alphabet without encoding (simpler inside the core, but every host with e-mail addresses or
paths as keys would invent its own escaping).

### 3. `:` becomes an ordinary character of a name

A raw `:` is no longer a separator. It joins the characters that stay as they are in an address, so the old address
`account/acme:7` stays valid and is read as the one name `acme:7`. A host that had two fields joins them with `:`
and reaches the same actors.

An old value that held a literal colon is stored as `%3A`. That is then no longer the canonical spelling:
`ActorAddress::parse` reads `a%3Ab` and writes `a:b`. Decision 4 says what happens to the stored text.

Not chosen: encoding `:` like any other special character (the migration would rewrite every address with two
parts); no migration at all.

### 4. The upgrade works in place: migration 3 removes every registration and rewrites `%3A`

Key fields leave every contract, so every contract hash changes. The migration removes every row of `actor_types`.
A node registers its types again when it starts, as it does for a new type, and the actors and their pending
messages stay. This is what 0.4.0 did for the formerly stateless types, now for all types.

Until a node that hosts a type starts, that type is not registered. For a host whose nodes all host the same types
there is no such window, because a node registers its types before it handles a message. For a host with different
kinds of nodes, in that window:

- the actors of the type can be read and deleted, and messages already in a mailbox wait;
- a create of one and a send to one from outside a turn are refused;
- a turn on another node that creates one ends `failed`, without a retry, and a raw send to one in a turn is
  refused.

The upgrade note says so, and says to start the nodes that host a type before the nodes whose turns create actors
of it, and nodes that only send last. Where two kinds of nodes create actors of each other's types, no order is
safe: the note says to hold back the messages that start such turns until every node is up.

The migration also rewrites `%3A` to `:` in the three address columns that SQL matches on: `actors.address`,
`messages.actor_address` and `outcomes.actor_address`, and only in rows that hold `%3A`. The other places that hold
an address (`messages.sender`, the `to` and `from` of a stored envelope) are read through `ActorAddress::parse`,
which gives the new spelling anyway, so they are left alone.

Two old keys with two fields can become one name, such as (`a:b`, `c`) and (`a`, `b:c`). The rewrite then meets the
primary key of `actors`, the migration fails with the database's own error for a key that exists, and the database
stays at version 2. Only `actors` has such a key: the outcomes of two actors that were deleted before would merge
under one address without an error. The upgrade note gives a query that finds such pairs before the upgrade, and says
to delete one actor of each pair with the old version; an actor cannot be renamed.

Not chosen: keeping the registrations, with `key` stripped and the hash blanked so that the first node overwrites
them (no window, but a rule in registration that exists only for this one upgrade); refusing a database that holds
`%3A` (needs more than the three statements of the rewrite); starting on a new database only.

### 5. What changes where

Decided by the main session; it follows from the proposal and the decisions above.

- **The address module.** `params.rs` gives way to a small reader built on serde (decision 1). `RefBinding` holds no
  list of fields. `:` joins the characters that `percent.rs` leaves as they are. `ActorAddress::of` takes a type
  name and one name, and `key_values()` gives way to `name()`, the decoded name. `to_params` and `from_params` go.
- **The round trip of the canonical check goes.** With one string there is one spelling, and
  `ActorAddress::parse` already writes it. What stays is that a node that hosts a type decodes an address as its ref
  and refuses one that the ref does not accept: a singleton accepts only `_`, and a ref with a reader of its own may
  refuse a name. The second half of `canonical_problem`, which writes the ref back and compares, and its special
  case in `turn.rs` go.
- **The contract.** `Contract` has no key fields: not in `new`, not in its JSON form, not in its hash. `params_of`
  and `key_fields()` go. `from_json` ignores a `key` entry, so a contract snapshot of an earlier version reads as the
  same contract and keeps passing; nothing is to be done about snapshots.
- **The wire model.** `TypeView.key`, `ActorView.params` and `ActorSnapshot.params` go; `docs/openapi.json` follows.
  The client, the columns and facts of the command line and the terminal UI follow.
- **The refs that change** in this repository: the four with two fields (`BatchItem` in the examples, three in
  tests) and the two with a number (`Num`, `Worker`). `BatchItem` keeps its typed fields and names `String` as its
  serde form, as the example of that way; the others get one `String`.
- **Migration 3** in both stores (decision 4), with tests on a version-2 database built by the helper that 0.4.0
  added. The upgrade tests of 0.4.0 that expect a registration to stay change with it.
- **Docs.** `README.md`, `docs/architecture.md`, `docs/operations.md` (schema version 3) and `CHANGELOG.md` with the
  upgrade note at the top of its section.

### 6. ADR 0021 records how an address is written

ADR 0018 decides that a key is one name. It does not say how a name is written, what a ref looks like, or what
becomes of the addresses that exist. `docs/adr/0021-a-key-is-one-name.md` records that, and says which sentences of
ADR 0013 it replaces. ADR 0013 gets one line under Status that points to it; its text stays. The main session wrote
ADR 0021 with this design and commits it with the change when it is ready.

Not chosen: no ADR, on the ground that ADR 0018 covers it (the project rule asks for one when a wire format
changes, and the spelling of an address is a rule that others must follow).

## Risks / Trade-offs

- **Every type is unregistered until a node that hosts it starts**, with the effects of decision 4; a turn that
  creates an actor of such a type fails for good. → Only with nodes that host different types. The upgrade note
  names the effects and the order to start nodes in.
- **Two old keys can become one name.** → The migration fails on the primary key and the database stays at version
  2. The upgrade note has a query that finds the pairs; the operator deletes one actor of each with the old version
  first.
- **A ref that changes its shape no longer decodes from what is stored.** Refs travel inside states and messages as
  JSON, such as `reply_to: Order` in the saga example. A ref that goes from two fields to one, or from a number to a
  string, is written differently, so a state or a pending message from before fails to decode and its turn ends
  `failed`. → The Migration Plan says so: let such actors finish and drain their mailboxes before the upgrade, or
  give the ref a reader that accepts the old form too. A ref that keeps its one string field, as Chorus's does, is
  not touched.
- **A listing prefix that holds `%3A` stops matching**, because the stored spelling is `:` now. → The upgrade note
  says to write `:`.
- **A host joins two parts in a way that is not unique**, such as a tenant and an id with a separator that a tenant
  name may hold. → That is the host's choice now; the README says to pick a separator the parts cannot hold.
- **A ref that is not one string compiles**, and fails when its node starts or at the first send. → The message
  names the ref and what serde wrote for it.
- **Migration 3 is not additive**: a node of the old version refuses the new contracts. → Every old node is stopped
  first, as for 0.4.0.
- **A client of the old version cannot decode the view of an actor or a type from a new node**, because its types
  require `params` and `key`. → The upgrade note says to upgrade clients together with the nodes.

## Migration Plan

For a host (Chorus, Karakuri):

1. A ref with two or more fields gets one string and joins its parts itself. To keep reaching the actors that
   exist, join with `:` in the old order of the fields. A field that is not a string becomes one. Such a ref is then
   written differently inside states and messages: let its actors finish and drain their mailboxes with the old
   version, or give the ref a reader that also accepts the old form.
2. Replace `ActorAddress::of(type, values)` with the form that takes one name, and `key_values()` with `name()`.
   Drop `to_params`, `from_params`, `RefBinding::key_fields`, the key argument of `Contract::new`,
   `Contract::key_fields`, `Contract::params_of`, `TypeView.key`, `ActorView.params` and `ActorSnapshot.params`.
3. Upgrade every user of `cicada-client` together with the nodes.
4. Stop every node of the old version.
5. Start the new version, the nodes that host a type before the nodes whose turns create actors of it, and nodes
   that only send last. The first node migrates the schema to version 3; each node registers its types.

Or start on a new database; then steps 1 to 3 are left.

There is no way back: a migrated database is not opened by the old version.

The version is a breaking one: 0.6.0 after 0.5.0.
