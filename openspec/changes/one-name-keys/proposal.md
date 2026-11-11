# Proposal

## Why

A ref is a struct whose named fields form the key: one value, or several joined with `:`, each percent-encoded (ADR
0013). That costs a serializer that turns fields into strings and back (`address/params.rs`), a check for the canonical
form of every value (`x/007` for an integer is refused), the names of the key fields in every contract and its hash,
and `params` in every actor view. Most refs have one field or none. ADR 0018 decides that a key is one name.

## What Changes

- **BREAKING** A ref holds one name. A ref without a name is a singleton with the key `_`, as today. A caller that
  needs two parts joins them itself (for example `b1.7`).
- `:` is no longer a separator inside a key.
- **BREAKING** A contract no longer lists key fields, so every contract hash changes. `TypeView.key` and
  `ActorView.params` leave the wire format.
- The text form stays `type/key` (ADR 0013). A listing prefix works as before.
- No namespace is added: an address keeps two parts (ADR 0018).

The simplest version: no typed parts of a key. A caller that wants a number in its key formats and parses it.

To decide in the design:

- How a ref is declared in code: a struct with one `String`, a newtype, or a helper.
- What a name may hold, and whether it is still percent-encoded.
- What happens to stored addresses whose key holds `:` or `%3A`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `actor-addressing`: the key of an address is one name, not several fields.
- `type-registry`: a contract, and so its hash, no longer holds key fields.

## Out of scope

- Namespaces or tenancy across types.
- The rules for type names.
- The paths of the HTTP API.

## Order

Step 3 of ADR 0018. After `private-store-trait`, before `tidy-the-core`.

## Impact

- `crates/cicada` (`address`, `contract`, `actor`, `turn`, `runtime`, `api`), `cicada-axum`, `cicada-client`,
  `cicada-cli` (the columns of an actor and a type), `cicada-test`, the examples, the sample service, `e2e`,
  `README.md`, `docs/architecture.md`, `docs/openapi.json` and `CHANGELOG.md`.
- ADR 0018 supersedes the keys of several fields in ADR 0013; no further ADR.
- Every contract hash changes, so nodes before and after must not share a database. Chorus and Karakuri must follow.
