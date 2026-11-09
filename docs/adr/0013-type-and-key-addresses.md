# 0013. Actors are addressed by type name and key

Date: 2026-11-09

## Status

Accepted

## Context

An actor's address was a URI bound to a template (`actor://sync/{tenant}`). The form came with the Kotlin port; no ADR
argued for it. Nearly every template in the repository was a name plus one id, or a singleton, so the hierarchy bought
almost nothing. It cost a scan of every registered template to find an actor's type, overlap rules between templates,
an escape for an actor whose last segment was `messages`, and addresses with slashes, which an OpenAPI path parameter
cannot hold. There are no production databases yet (ADR 0008).

## Decision

An actor is addressed by a **type name** and a **key**. The text form is `type/key` (`sync/acme`), used in messages,
state, listings and as the path below `/actors/` over HTTP.

- A type name matches `[a-z][a-z0-9._-]*`, at most 100 characters. A versioned type is a name such as
  `billing.v2.invoices`.
- `ActorRef` declares `const TYPE`. Refs stay typed structs: their fields form the key, one value (`acme`) or several
  joined with `:` in declared order (`b1:7`). Each value is percent-encoded, so a key holds no raw `/`, and a `:` is
  always a separator. A unit struct is a singleton with the key `_`. The key `.` or `..` is refused.
- The address carries its type, so finding the type is a direct lookup and templates cannot overlap.
- A contract holds the type name and the names of the key's fields, so changing a ref's fields changes its hash.
- A listing's `prefix` is a plain prefix of the text form: `sync/` is one type, `sync/ac` a key prefix.

Over HTTP, an actor is `/actors/{type}/{key}` and its mailbox `/actors/{type}/{key}/messages`. The view of an actor has
`address`, `type` and `params`; the view of a type has `name` and `key`.

## Consequences

- Every use of `ActorRef`, `ActorAddress` and the HTTP paths changes, and every contract hash. A database written by
  an earlier version cannot be read; the store schema changes in place at version 1.
- `UriTemplate`, the overlap conflict at registration, and the check that a typed send's address belongs to its ref's
  type are gone.
- A hierarchy of names is only a naming habit now. Prefix search still works on the text form.
- Guarantees G1–G6 and store requirements S1–S10 do not change.
