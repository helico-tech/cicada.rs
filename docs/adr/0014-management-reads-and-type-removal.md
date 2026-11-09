# 0014. Management reads and type removal

Date: 2026-11-09

## Status

Accepted

## Context

A tool that watches or repairs a deployment could only list failed messages, had no cheap count of actors and
mailboxes, could not find what a message caused (0.2.0 removed the chain walk, ADR 0012) and could not remove a
type's registration without SQL. ADR 0008 allows changes to the wire format and the schema until there is a
production database. The changes are `outcomes-listing`, `store-counts`, `messages-by-cause` and
`remove-registered-type` in `openspec/`.

## Decision

- `GET /outcomes?status=&prefix=&since=&cursor=&limit=` replaces `GET /failures`. `status` is `done`, `failed`,
  `cancelled` or `purged`; absent means all; anything else is `400`. Order, retention and hidden records are as for
  failures. `Store::failures` becomes `Store::outcomes`.
- `GET /stats?prefix=` answers `{actors, pending, oldestDue}`. It is a plain count over the rows under the prefix and
  is not a snapshot across its two reads. `oldestDue` is the earliest `deliverAt` of a pending message that is due at
  store time, or null.
- `GET /messages/{id}/caused?cursor=&limit=` lists one level of messages whose `causedBy` is `id`: pending or running
  ones and retained visible outcomes, as message statuses in message-id order. `404 no-such-message` when `id` is
  unknown or past retention. The schema (version 1, in place) gets back the index on `messages.caused_by` and the
  column `outcomes.caused_by` with its index.
- `DELETE /types/{name}` removes a registration: `204`, `404 no-such-type`, or `409 type-in-use` while actors of the
  type exist. The check and the removal are one atomic step. PostgreSQL takes the row lock of the registration for
  the removal, and `create` takes a share lock on it; `ActorSystem::create_actor` looks the type up again after the
  insert and takes the new actor back if the type is gone. A node that still hosts the type registers it again at its
  next start, so the operations guide says to stop such nodes first.

## Consequences

- Callers of `/failures`, `Store::failures` and the matching `ActorSystem` and `ActorClient` methods move to
  `outcomes`; `CHANGELOG.md` says so.
- `Store` has three more methods (`stats`, `caused_by`, `remove_type`); the conformance suite covers them.
- Creating an actor in a commit (`turn.create`) does not take part in the type lock: a type removed while a node still
  runs turns for it can get an actor behind it. That is the case the guide tells operators to avoid.
