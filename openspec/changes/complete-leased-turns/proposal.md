# Proposal

## Why

A leased actor turn must submit all of its effects in one fenced transaction. The server needs a handler report,
not permission for the client to choose arbitrary message outcomes or bypass retry and validation rules.

## What Changes

- Add completion reports for returned, failed and cancelled handlers to the public lease API.
- Apply state, sends, creates, cancels, watches and controls through the existing atomic commit path.
- Share validation and failure-to-retry decisions with the local executor; derive sender and caused-by identity.
- Define stale epochs, rejected effects, repeated reports, lost acknowledgements and failure fallback.

## To decide

- Complete report/result shapes, retryable failures and cause validation; distinguish omitted state from null.
- Every current Turn operation's checks, registry lookups, effect order, generated ids and message retention.
- Atomic validation against the claimed message/epoch; server reads needed to derive retry from stored policy.
- Retrying an unchanged report, rejected-commit fallback and the G6 boundary after a handler has ended.
- Preserve safe uncertainty after a lost response unless evidence requires durable completion receipts.

## Out of scope

HTTP transport, worker scheduling, a guarantee of exactly-once external effects, and a separate public Backend API.

## Order

After `lease-turns-in-core`, before `remote-worker-runtime`.

## Impact

Core completion types/operation, shared executor policy and validation, effect and failure tests, architecture,
guarantee test references and the remote-turn ADR. Reconcile all names against the landed predecessor.
