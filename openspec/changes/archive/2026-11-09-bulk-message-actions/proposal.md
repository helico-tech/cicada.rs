# Proposal

Status: built on 2026-11-09.

## Why

After an incident an operator may need to act on many messages at once. Oban Web offers cancel and retry on a selection
or on everything matching a filter, capped at 1,000 by default.

## What Changes

- `POST /messages/bulk-cancel` and `POST /messages/bulk-retry` take explicit ids or a filter (address prefix, message
  type).
- At most 1,000 messages per call. The answer gives the number affected and whether more match.

Each message gets the same treatment as the single action. A call is not one transaction, so a call that fails half way
has done part of its work; the count says how much.

## Out of scope

- Background jobs for larger sets. Call again until nothing matches.
- Bulk actions on actors.

## Impact

- No new store methods: built in `ActorSystem` on the single operations and the existing listings.
- `cicada-axum`, `cicada-client`.
- Changes the wire format, so it needs an ADR.
