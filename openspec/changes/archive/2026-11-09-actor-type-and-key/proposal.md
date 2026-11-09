# Proposal

Status: agreed by Arjan on 2026-11-09 (option 2 of the discussion on actor URIs). Built on 2026-11-09.

## Why

An actor's address is a URI bound to a template (`actor://sync/{tenant}`). The form came with the Kotlin port; no ADR
argues for it. Of the 22 templates in the repo, nearly all are a name plus one id or a singleton, and one has two
parameters, so the hierarchy buys almost nothing. It costs:

- Finding an actor's type by loading every registered template and matching the address against each.
- Overlap rules between templates, at node start and at registration.
- The `%6Dessages` escape for an actor whose last segment is `messages`.
- Addresses with slashes. A slash cannot be in an OpenAPI path parameter, and axum cannot put sub-paths after a
  catch-all, so the management API would need percent-encoded `%2F` segments, which some proxies mishandle.

There are no production databases yet (ADR 0008), so this is the cheapest moment to change it.

## What Changes

- An actor is addressed by a **type name** and a **key**. `ActorRef` declares `const TYPE: &'static str` instead of
  `const TEMPLATE`. A type name matches `[a-z][a-z0-9._-]*`.
- Refs stay typed structs, so the compiler still checks that a ref accepts a message. The fields become the key:
  one value (`acme`), or several joined with `:` in declared order (`b1:7`). Each value is percent-encoded, so a key
  never holds a raw `/` or a raw `:` from a value. A unit struct is a singleton with the fixed key `_`.
- `ActorAddress` holds type and key. Its one text form is `type/key` (`sync/acme`), used in `to`, `from`, state and
  stored refs. It needs no scheme.
- The address carries its type, so finding the type is a direct lookup. The check that a typed send's address belongs
  to the type of its ref goes away.
- A contract holds the type name and the key's field names, so changing a ref's fields changes the contract hash.
- Store: `actor_types` is keyed by name and `shape` goes. The address carries the type, so `create` loses its template
  argument and records lose their template field. The schema changes in place at version 1 (ADR 0008).
- Listings keep `prefix`: a plain prefix of the text form, so `sync/` is one type and `sync/ac` a key prefix.
- HTTP paths become `/actors/{type}/{key}` and `/actors/{type}/{key}/messages`. `params` stays in the actor view, by
  field name.

Removed: `UriTemplate` and its parsing, matching, expansion and overlap logic, `support::resolve` and `ResolvedType`,
the overlap conflict at registration, `ActorAddress::to_http_path` and `from_http_path`, and the mailbox-segment
escape. The estimate is 300 to 400 lines of about 1,050 in `crates/cicada/src/address/`; the gain is mostly fewer
concepts.

## Out of scope

- New endpoints; those are `management-api` and the changes after it.
- Hierarchical names. A versioned type is a name such as `billing.v2.invoices`.
- Dropping typed refs for plain string keys.
- Migrating existing databases.

## Impact

- Breaking for every user of `ActorRef`, `ActorAddress`, the HTTP paths and the stored form of addresses. Changes the
  wire format and the contract hash, so it needs an ADR and a new `CHANGELOG.md` entry.
- `crates/cicada` (`address`, `actor`, `contract`, `runtime`, `store`, `api`), all three stores and the conformance
  suite, `cicada-axum`, `cicada-client`, `cicada-extras`, `examples`, `sample-service` (recorded contracts and state
  samples), `e2e`.
- `README.md`, `docs/architecture.md` (domain language), `docs/operations.md` (versioned templates).
- Guarantees G1-G6 and S1-S10 do not change.
