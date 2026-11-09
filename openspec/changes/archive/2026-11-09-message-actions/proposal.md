# Proposal

Status: built on 2026-11-09.

## Why

An operator cannot retry a failed message directly, or run a delayed one now. Today a retry means re-sending by hand
with a new id, and a delayed message can only be cancelled.

## What Changes

- `POST /messages/{id}/cancel` replaces `DELETE /messages/{id}`. For a running message it answers as today (`409`);
  stopping a running turn is `turn-management`.
- `POST /messages/{id}/retry` sends a failed or cancelled message again: a new message with a new id, the same target,
  type, payload, correlation id and headers, and `causedBy` set to the original. Answers an explicit error for a message
  that is still pending or that ended `done`, instead of doing nothing.
- `POST /messages/{id}/deliver-now` makes a pending message due at store now. It takes a new place in delivery order, as
  a rescheduled message does.

## Out of scope

- Retrying a message whose record is past retention or was removed by its policy: its payload is gone.
- Editing a message.

## Impact

- New store methods in all three stores and new conformance cases.
- `cicada-axum`, `cicada-client`. `cicada-extras`: `Redrive` in the dead letters stays as it is.
- Changes the wire format, so it needs an ADR.
