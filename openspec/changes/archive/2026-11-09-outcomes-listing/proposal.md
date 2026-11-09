# Proposal

Status: built on 2026-11-09 (agreed by Arjan on 2026-11-09). Depends on
`management-api`.

## Why

Only failed messages can be listed. To see what an actor did, or any finished message that is not failed, you must
already know its id.

## What Changes

- `GET /outcomes?status=&prefix=&since=&cursor=&limit=` lists finished messages, by any status.
- It replaces `GET /failures`: failures are `status=failed`.
- `Store::failures` widens to take a status.

## Out of scope

- Records hidden by a message's retention policy, and anything past retention.
- Full-text search in payloads.

## Impact

- `Store::failures` changes in all three stores, and the conformance cases follow. An index may be needed for the status
  filter.
- `cicada-axum`, `cicada-client`, `cicada-extras` (`FailureWatcher`).
- Changes the wire format, so it needs an ADR.
