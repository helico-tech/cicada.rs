# Proposal

## Why

Systems such as GitHub, Jira, Stripe and Slack report a change by calling a URL of the host (a webhook). To turn that
call into a message for an actor, a host writes a route, checks the signature, drops the repeats the sender makes
after a timeout, and sends the message. These steps are the same for every sender and easy to get wrong: a missing
signature check lets anyone inject messages.

This is the third of three generic integration actors, after `webhook-sink` and `http-poller`. Strictly it is a
route, not an actor: it is the way in for later vendor actors that react to events.

## What Changes

- **A webhook route** for `cicada-axum`: `POST` to a path that ends in a hook name. The host registers each hook with
  a target address, the message that wraps the body, the name of its secret, and how the sender signs.
- **Signature check first.** The route computes an HMAC (a keyed hash) of the raw body with the hook's secret and
  compares it, in constant time, with the sender's header. A missing or wrong signature answers `401` and sends
  nothing. A hook without a secret cannot be registered. The design decides which header and hash formats are
  supported at first; GitHub's `X-Hub-Signature-256` is the reference case.
- **Duplicates are dropped by the store.** The route derives the message id from the hook name and the sender's
  delivery id header. Message ids are unique within the dedup window (S7, S10), so a repeated delivery sends nothing
  new and still answers success. The design decides what happens when a sender gives no delivery id; a hash of the
  body is the likely answer.
- **Answer after the commit.** The route answers `2xx` only when the message is stored, so a sender that retries on
  any other answer loses nothing.
- **Subscribing.** As with the poller, the target is one address, and a topic serves several interested actors.
- It reuses the secret resolver of `webhook-sink`.

## Out of scope

Parsing a vendor's events into typed messages: the body is passed on as JSON. Senders that sign with something other
than an HMAC of the body (mutual TLS, signed tokens). Answering the sender with data from an actor, as Slack's
interactive messages need. Limiting how fast a sender may call. Protection against replays older than the dedup
window.

## Order

After `http-poller`. No following change.

## Impact

A new router in `cicada-axum` with tests, `docs/openapi.json` if the design lists the route there, a worked example,
and a handoff to the docs session for the HTTP guide. It adds an unauthenticated public route to a node that mounts
it, guarded only by the signature: the design states that risk and whether an ADR is needed. No change to the core,
the store, a guarantee or an existing wire format.
