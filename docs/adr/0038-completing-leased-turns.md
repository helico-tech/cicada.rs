# 0038. Completing leased turns

Date: 2026-11-13

## Status

Accepted for `complete-leased-turns`. The owner delegated technical decisions to an Opus 5.5 adviser. The design
retains server-derived policy and makes the rejected-to-failed transition explicit at the caller to preserve retries.

## Context

ADR 0037 leases turns without hosting their handlers. Those trusted executors next need to submit a finished handler
report through any server. The existing store commit already supplies atomic effects and fencing. Exposing that
private request would let callers choose outcomes and sender/event identities; duplicating the runtime would drift.

## Decision

- Add complete_turn(actor, epoch, message_id, report), with Returned, Failed and Cancelled reports and public plain
  effect records. Keep private store/executor request types private. The server derives sender and caused-by and
  creates the notification event id; send ids remain fixed in the caller's report.
- Share run-report, retry, state validation, request building and rejection policy with the local executor. Its
  local behavior stays unchanged. The remote path checks storage limits but does not run a handler, apply new JSON
  Schema rules or depend on the server's hosted types. Executor-side type/watch/buffering checks remain its duty.
- Derive retries from a stored Pending or Running message of this actor. Policy/failure count cannot change while
  the same epoch remains held without a commit or an operation that voids the lease; the final fence protects the
  read/commit race. The snapshot is not a validity check, and expired-but-current epochs may still commit as before.
- Make at most one commit attempt per call. Confirmed application returns Completed; lost epoch returns LeaseLost;
  uncertain errors return without changing the report or releasing. There is no server-local session or receipt.
- A known rejection returns Rejected with the exact nonretryable TurnFailure selected by the shared fallback policy.
  The caller replaces its report with that Failed report and thereafter retries only it. A refused terminal failure
  is an invalid request, not another fallback. No server internally attempts a different fallback commit.
- The extra round trip after rejection is intentional: an internal fallback followed by a lost response would let
  a peer reconsider the original effects, despite the caller not knowing the failure decision. A lost rejection
  response made no fallback attempt, so the original report may still be re-evaluated, as with an unknown store reply.
- Completion uses no server config/initialization/phase, works after node stop while Store is open, and never calls
  release or server observers. The worker owns cancellation arbitration, conservative deadlines, renewal and G6.

## Consequences

- The same unchanged Rust handlers can later run under Worker; this primitive adds neither Worker nor HTTP routes.
  A non-Rust executor must obey the same finished-buffer, type-check, cancellation and after-handler duties.
- Retryable Handler/Node/Timeout reports use the message's policy; permanent/decode/state/commit failures do not.
  ClaimLimit is not a caller-selected cause. Invalid new own state keeps the current Handler failure classification.
- A repeated commit after application returns LeaseLost, which does not distinguish success from replacement.
  Retention-hidden outcomes and rescheduled messages mean a later GET cannot always prove what happened. Only
  acknowledged Completed ends a worker observer; abandoned/uncertain turns drop it.
- Repeating the current report is safe but repeating a finished handler is not a commit retry. A caller that has
  learned of rejection must never restore its original effects. External side effects remain at least once.
- No database migration, Backend behavior change, receipt-retention policy or weaker delivery guarantee. New tests
  cover fencing, effects once, uncertainty and the explicit fallback transition and are linked from the guarantee table.
