# 0036. Registering contracts without hosting their actors

Date: 2026-11-13

## Status

Accepted for `register-contracts-over-http`. The owner chose remote execution of existing Rust actors over a
language-neutral HTTP protocol, with lease operations independent of the HTTP server that issued them.

## Context

An HTTP server for remote workers need not contain their handlers. It still needs their contracts to accept creates
and messages. Today registration happens when a local node initializes. The private store already supports atomic
registration by hash, and ADR 0017 specifies equal contracts and preservation of the first descriptions.

## Decision

- Add ActorSystem::register_contract independently of node initialization or hosted handlers. Recompute the hash
  from its name and schemas rather than trust a hash carried by a Contract loaded from a TypeDef.
- Expose PUT /types/{name} on the full router. Its strict body contains only required state and messages fields.
  The path supplies the type name. Return 200 and the stored TypeView for both new and equal registrations.
- Preserve atomic first-registration-wins semantics. A different hash is 409 contract-conflict, with the type and
  both hashes in the ordinary Problem detail; no replacement even when the type currently has no actors.
- Validate schema roots as objects or booleans, message names against the existing send limits, and all schema
  strings/keys for NUL. Bound each complete schema at 120 nested arrays/objects; a scalar counts zero. Storage adds
  at most two levels and the type-list response four, so both fit existing 128-level parsers. Invalid contracts are
  422 invalid-contract. Malformed bodies and paths remain 400.
- Do not compile schemas or retrieve references during registration. ADR 0026's usable-schema checks and fallback
  stay in force at the create/send door. The 64-level actor-value limit is separate from the new schema bound; HTTP
  keeps its existing parser and body limits. Local generated contracts and initialization keep their behavior.
- Reuse parsed resolver entries only when both their hash and complete source JSON match the stored definition.
  A hash alone cannot distinguish descriptions replaced after removal, including removals made by another system.
- Add ActorClient::register_type returning the stored TypeView or the existing client problem error. It transmits
  no hash. The read-only router does not gain a write operation.

## Consequences

- A host can publish contracts, create actors and queue messages before a matching implementation connects.
  Registration itself supplies no executor, worker membership, actor-change hint or node row.
- A later local or remote worker must produce the same contract hash, including schema details other than the
  descriptions excluded by ADR 0017. Handwritten approximations are not compatible with generated contracts.
- Schema documents remain trusted registration data. Acceptance is not a guarantee that a handler exists, that
  schemas compile in every validator, or that any value satisfies them. Host authentication covers this write route.
- Deleting an unused registration and registering again remains the existing explicit replacement mechanism.
  No hot reload, schema migration, store-trait exposure, new dependency or change to G1–G6/S1–S10 is introduced.
- Lost HTTP responses can be retried with the same contract. Equal retries return the stored descriptions, not the
  latest submitted wording. No new mutable metadata is added.
- A generated schema above 120 levels must be simplified to use remote registration. Older over-deep registrations
  written by local hosting are not repaired by this change; the new entry point never adds one.
