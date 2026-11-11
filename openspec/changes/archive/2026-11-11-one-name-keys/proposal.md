# Proposal

## Why

A ref is a struct whose named fields form the key: one value, or several joined with `:`, each percent-encoded (ADR
0013). That costs a serializer that turns fields into strings and back (`address/params.rs`), a check for the canonical
form of every value (`x/007` for an integer is refused), the names of the key fields in every contract and its hash,
and `params` in every actor view. Most refs have one field or none. ADR 0018 decides that a key is one name.

## What Changes

- **BREAKING** A ref holds one name. It is a type that serde writes as one string (a struct with one string field,
  or a type that serde writes as a string itself) or as a unit, for a singleton with the key `_`, as today. Only a
  string counts: a number, a single character or an enum variant as the field is refused. A caller that needs two
  parts joins them itself. A ref that is not one string stops its node from starting.
- A name may hold any text and stays percent-encoded in an address, as today.
- `:` is no longer a separator inside a key. It is an ordinary character of a name and is written as it is, so an
  old address such as `account/acme:7` stays valid as the one name `acme:7`.
- **BREAKING** A contract no longer lists key fields, so every contract hash changes. `TypeView.key` and
  `ActorView.params` leave the wire format.
- An appended migration removes every registration, so that nodes register their types again and a database is
  upgraded in place. It also rewrites `%3A` to `:` in the three address columns that SQL matches on.
- The text form stays `type/key` (ADR 0013). A listing prefix works as before.
- No namespace is added: an address keeps two parts (ADR 0018).

The simplest version: no typed parts of a key. A caller that wants a number in its key formats and parses it.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `actor-addressing`: the key of an address is one name, not several fields.
- `type-registry`: a contract, and so its hash, no longer holds key fields; every registration is removed when an
  older database is opened.
- `actor-state`: an old database ends at the schema version of the store that opened it, not at version 2.

## Out of scope

- Namespaces or tenancy across types.
- The rules for type names.
- The paths of the HTTP API.

## Order

Step 3 of ADR 0018. After `postgres-tls`, before `tidy-the-core`.

## Impact

- `crates/cicada` (`address`, `contract`, `actor`, `turn`, `runtime`, `api`, both stores and their migrations),
  `cicada-axum`, `cicada-client`,
  `cicada-cli` (the columns of an actor and a type), `cicada-test`, the examples, the sample service, `e2e`,
  `README.md`, `docs/architecture.md`, `docs/openapi.json` and `CHANGELOG.md`.
- ADR 0018 supersedes the keys of several fields in ADR 0013. ADR 0021 records how an address is written from then
  on, and ADR 0013 gets a line under Status that points to it.
- Every contract hash changes, so nodes before and after must not share a database: every old node is stopped first,
  and the database is upgraded in place (`design.md`, decision 4). Chorus and Karakuri must follow.
