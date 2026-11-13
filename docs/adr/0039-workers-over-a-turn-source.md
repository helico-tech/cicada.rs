# 0039. Workers over a turn source

Date: 2026-11-13

## Status

Accepted for `remote-worker-runtime`. The owner chose unchanged Rust actors first over a language-neutral remote
protocol and delegated technical decisions to an Opus 5.5 adviser.

## Context

ADRs 0037 and 0038 provide handler-free lease/completion primitives. A Rust executor must reuse Actor and Turn,
including their cancellation and G6 rules, without requiring a Store or duplicating the runtime. HTTP is the next
change; an in-process source and fault wrappers are sufficient to define and test Worker now.

## Decision

- Publish async TurnSource over the existing public core registration/type/lease/complete models, with reference
  ActorSystem and Arc implementations. SourceError distinguishes Unavailable, Invalid and Fatal. No public Backend,
  wire DTO, HTTP dependency or remote management facade is introduced.
- Add Worker with host/observe/config, initialization/start/manual batches, stop and a stopped waiter that reports
  the first automatic-stop error. Preserve existing actor definition/hash checks; accept at most 100 hosted types.
  Registration can be retried but a stopped worker cannot restart. Worker never closes or stops its source.
- Share TurnExecutor and handler-phase logic, using private store/source holders, per-turn timing and report output.
  The local store path keeps its behavior. TypeResolver similarly reads from store or source with its existing
  parsed-contract cache; a remote lookup remains a fresh source read rather than new registry caching.
- Use server-granted TTL, conservative request-start deadlines and nonshortening renewal. Derive heartbeat from
  TTL/3 with a one-nanosecond floor for valid grants, rather than a floor longer than a valid short TTL. Renew has an
  independent watchdog. Frozen/estimated handler times keep their existing formula with worker request-start time.
  Check expiration/loss before accepting renewal or starting completion; a late reply cannot revive a local grant.
- Reserve capacity across lease requests and turn tasks, shared by background and manual callers. Track dropped
  manual cohorts through cleanup. Stop must drain manual work even while NodeState is New. Registration awaits must
  not prevent stop from marking a new Worker stopped; a late start cannot dispatch afterward.
- Keep one current completion report. Known Rejected switches it permanently to the provided nonretryable failure,
  including when a retryable Failed report was refused while rescheduling. Never retry the original effects again.
  Capture one completion budget; race each pending call with that budget and lease loss, renewing while it runs.
- Complete using the server's actual outcome; end observers only on acknowledged Completed. Unknown results,
  LeaseLost and exhausted/permanently refused attempts abandon without release. GET cannot prove every outcome.
- A permanent lease-path failure initiates graceful automatic stop; per-turn source errors retain the documented
  deadline/abandonment behavior. Worker has no node-table membership, store sweep, TTL/heartbeat override or local
  claim-limit policy. Server and Worker lifecycle are independent.
- Operator cancellation received through renewal wins by the shared handler phase and completes Cancelled even if
  shutdown follows. Shutdown/manual cancellation alone releases only stopped work whose handler has not ended.

## Consequences

- Existing handlers, raw sends/creates, controls, watches, MAX_TURN, panics and observer/OTel headers work under the
  same executor. HTTP and CLI adapters can follow without changing the actor programming model.
- Source implementations are trusted to uphold core fencing/claim guarantees. Local metadata checks catch malformed
  batches but do not make a dishonest source safe. HTTP adapters must classify errors and avoid hidden mutation retries.
- Lost lease answers and failed cleanup can leave leases to expire. Cooperative cancellation cannot forcibly stop
  blocking code, and the worker may return from stop while such code remains; it must not release under it.
- Manual and background scheduling share configured capacity. An empty Worker is valid and never leases. Automatic
  failure is observable through stopped, so the future CLI can exit with an error rather than quietly idle forever.
- No schema change, weakened guarantee or new dependency. Parity and fault tests cover the shared executor and
  explicit completion transition; new guarantee tests are added to their table.
