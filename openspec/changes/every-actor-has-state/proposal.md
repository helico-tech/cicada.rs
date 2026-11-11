# Proposal

## Why

An actor type is either stateful or stateless, and the difference buys nothing. It costs a trait (`ActorState`) whose
only job is a `STATELESS` flag, an optional state in the store model, the commit, the wire model and the contract, and
two create methods on both `Turn` and `ActorSystem`. In this repository only three examples and some test actors are
stateless; 41 actor types have state. ADR 0018 decides that every actor has state.

## What Changes

- **BREAKING** `Stateless`, the `ActorState` trait and `create_stateless` (on `Turn` and `ActorSystem`) are removed.
  `Actor::State` is any `Serialize + DeserializeOwned + JsonSchema` type.
- The core gets `NoState`, an empty struct with braces that refuses unknown fields. It encodes to `{}`.
- **BREAKING** State is never absent: not in the store (the column is not null), not in a commit, not in a contract
  (`state_schema` is always there), not over HTTP (an actor's `state` is always an object or another JSON value, never
  null or missing).
- An appended migration sets the state of existing rows without one to `{}`, makes the column not null and removes
  the registration of every type without a state schema, in both stores. A node registers such a type again at
  start, so a database is upgraded in place.
- **BREAKING** `PUT /actors/{type}/{key}` and `cicada create actor` need a state. A state that is JSON `null` is
  refused everywhere.
- **BREAKING** The contract hash of every type that was stateless changes, because its contract gains a state schema.

The simplest version: no compatibility shim and no alias for `Stateless`. Callers replace `Stateless` with `NoState`
and `create_stateless(&r)` with `create(&r, &NoState {})`.

## Capabilities

### New Capabilities

- `actor-state`: every actor instance has a state value, and every contract a state schema.

### Modified Capabilities

- `type-registry`: a registered type always has a state schema.

## Out of scope

- A `Default` bound on state, or creating an instance without naming its state.
- Changing how state is decoded or evolved (`#[serde(default)]` stays the rule).
- The other steps of ADR 0018 (keys, the store trait, watches and the rest).

## Order

Step 1 of ADR 0018. After `fix-backends-commit-difference`, before `private-store-trait`.

## Impact

- `crates/cicada` (`message`, `actor`, `turn`, `contract`, `store::model`, `runtime`, `api`), both store backends and
  their migrations, `cicada-store-conformance`, `cicada-test`, `cicada-axum`, `cicada-client`, `cicada-cli`,
  `cicada-fuzz`, the examples, the sample service, `e2e`, `README.md`, `docs/architecture.md`, `docs/operations.md`,
  `docs/openapi.json` and `CHANGELOG.md`.
- Covered by ADR 0018, which becomes Accepted with this change; no further ADR.
- Chorus and Karakuri must rename `Stateless` and drop `create_stateless`. A database written before this change is
  migrated in place (design, decision 1). Nodes before and after must not share a database: every old node is stopped
  first.
