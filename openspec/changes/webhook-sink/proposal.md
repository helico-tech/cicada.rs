# Proposal

## Why

Cicada's ready-made actors only talk to other actors. A host that wants to tell an outside system something (a Slack
channel, a Teams channel, an incident tool, its own service) writes the HTTP call, the retries and the duplicate
handling by hand each time. Most of these systems accept a JSON `POST` to a URL, so one generic actor covers them and
is the base for later vendor actors.

This is the first of three generic integration actors: `webhook-sink` (outbound), `http-poller` (inbound by polling)
and `webhook-inbox` (inbound by webhook). The owner chose to build these before any vendor actor. This change also
settles two questions the other two reuse: where secrets live, and where the HTTP client dependency lives.

## What Changes

- **A webhook sink actor** (`cicada.webhook-sink`), one instance per endpoint. Its state holds the URL, fixed headers
  and the *name* of a secret. Its public message carries a JSON body. A turn sends one `POST` and commits when the
  endpoint answers with a success status.
- **Failures use the existing rules.** A failed call fails the turn, so the message's retry policy decides the next
  attempt and a watch (dead letters) sees the final failure. The design decides which statuses are final (most 4xx)
  and whether a `429` or `Retry-After` delays the retry instead of counting as an attempt.
- **Duplicates.** A call is at-least-once (G4). The sink sends the message id as an `Idempotency-Key` header, the same
  on every attempt, as `docs/architecture.md` advises. An endpoint that ignores the header can receive a repeat; the
  guide says so.
- **Secrets stay out of the store.** State is rows in SQL, so it holds a secret's name only. The host gives the actor
  a resolver from name to value when it registers the type. The design decides the shape of that resolver and how the
  value is placed (a header, or a part of the URL, as Slack's incoming webhooks need).
- **Where it lives.** A new crate for integration actors, so that `cicada-actors` stays free of an HTTP client (the
  owner's choice). The design names the crate. The contract goes to `docs/reference/contracts/` like the others.

## Out of scope

Vendor-specific actors and message formats (Slack blocks, Teams cards). Methods other than `POST` and bodies other
than JSON. Reading the response body or replying to the sender with it. OAuth token refresh. A strict outgoing rate
limit: a host puts a rate limiter in front. Polling and incoming webhooks, which are the next two changes.

## Order

After `local-push-gate`. Before `http-poller`.

## Impact

A new actor with its contract, tests on virtual time against a fake endpoint, a worked example, and a handoff to the
docs session for the actors guide. A new workspace crate with an HTTP client dependency, and its rows in AGENTS.md
and `docs/architecture.md`. The design says whether
an ADR is needed for the secret resolver. No change to the core, the store, a guarantee or an existing wire format.
