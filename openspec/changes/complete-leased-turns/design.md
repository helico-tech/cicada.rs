# Design

## Public reports

Add `ActorSystem::complete_turn(&ActorAddress, epoch: i64, &MessageId, TurnReport)` returning
`Result<CompleteTurnResult, OpError>`. Public runtime types are cloneable/debuggable/PartialEq and constructible:
- TurnReport: `Returned { state: StateReport, effects: TurnEffects }`, `Failed(TurnFailure)`, or `Cancelled`.
- StateReport: `Unchanged`, `Set(Value)`, `Stop`; Set(null) is distinct from no change. Stop omits an encoded state.
- TurnEffects (Default): watches, creates, sends, cancels and controls in their existing vector order.
- TurnSend: to, message_type, payload, id, delivery, correlation_id, headers, retry and retention; no from/caused_by.
- TurnFailure: cause (FailureCause), retryable (bool), reason (String).
- CompleteTurnResult: `Completed(TurnCompletion)`, `Rejected(TurnFailure)`, or `LeaseLost`.
- TurnCompletion: outcome (TurnOutcome), cause (Some only for failed), version, sent messages and cancelled ids.

Re-export the existing ActorCreate/Control/ControlEffect records from store and make watch::WatchChange public;
TurnEffects uses them. Keep Backend, Claim, CommitRequest, CommitResult, OutgoingMessage and EffectBuffer private.
No serde wire DTO, transport trait or worker dispatcher here. The report is a finished effect buffer, not a log of
Turn calls: callers preserve the existing buffering rules such as cancel removing a buffered send and last control
winning. A raw report with both a send and its cancellation gets backend order (cancel before send), not normalization.

## Shared policy, separate execution

Extract private helpers for run-result-to-report, report-to-Decision, sanitization/request building and selection of
the failure report after a known refusal. The local executor uses them but retains its existing commit retry and
fallback loops, lease guard, logging, observers and control flow. Keep its behavior and existing tests unchanged.
The server derives an outcome; no report supplies TurnOutcome, retry policy/failure count of the input, or event id.

Retryable=true is allowed only for Handler, Node and Timeout. Retryable=false additionally allows Refused, Message,
State and Commit. ClaimLimit is never accepted from a caller. Other combinations are OpError::Invalid before store
access. A failed/cancelled report carries no user effects by construction. Permanent handler errors map to Refused;
panics inside handlers to Handler; outer execution panics to nonretryable Node; undecodable messages to Message;
load failures to State; timeout retryability and errors after handler end keep the executor's present rules.

For retryable failure, read status(message_id) once and use retry/failures from either Pending or Running when its
message targets this actor. Other states supply no retry; the atomic commit still decides fencing/identity. While
this epoch remains held, retry and failures cannot change without a commit or an operation that voids the lease.
Thus the read need not be atomic with commit; a concurrent replacement cannot apply a decision under the old epoch.
Use failures+1, retries_after and capped backoff exactly as handler_failed. A read error returns OpError::Store
without committing. Never infer LeaseLost from a running-turn view: it hides expired but still held leases.

## Validation and identities

Before commit, the remote path checks each send with existing delivery/envelope checks and each create with null,
NUL and value-depth checks. This backstop maps refusal to a no-effects Failed(Commit), prefixed `Commit refused:`.
The shared own-state check maps an invalid Set value to nonretryable Handler with `State cannot be stored:`, as
after_return does today. State encoding failures are reported as nonretryable Handler by the executor. Stop wins
over a new own state in that conversion; it does not skip validation of other effects.

Do not compile JSON Schema or decode target state on the server. Encoding, raw registry lookups, hosted-target
address/state checks, post-handler typed-effect checks, watch permission, observer header merging and call-order
normalization remain executor duties. The server's own hosted types must not change completion behavior. Incoming
RetryPolicy values retain their constructor validation; do not add new retention/watch constraints beyond Turn's.

Set each send's from to actor and caused_by to message_id; keep its caller-generated id stable across retries.
Generate a fresh commit event id per call: only one epoch can commit, so only that event's notifications exist.
Backend commit still checks the claimed message id, actor life/epoch, self-cancel, conflicting child/control effects,
targets, ids and ancestor deletion. Some structural checks precede fencing: do not promise LeaseLost overrides all
invalid reports. A wrong message id never applies a report, including a later failed report with that same wrong id.

## One attempt and an explicit fallback

complete_turn makes at most one commit attempt per call. On success return the actual outcome/cause, last version
(before removal for Stop), sent envelopes and cancelled ids; wake local dispatch and log committed controls.
On LeaseLost return LeaseLost. On Unavailable/Corrupt return OpError::Store, wake conservatively for an uncertain
commit, and make no different commit. The caller retries its identical current report within its deadline while
renewing; it never reruns the handler or releases after handler end. No config read, initialization, observer or
server-local session is involved; completion works after node stop and on any peer while the store is open.

On a known Rejected or StoreError::Invalid, use the local executor's fallback selector and return Rejected with a
nonretryable TurnFailure; no failure is committed in that call. Normally cause Commit, preserving local error text;
a refused reschedule retains the original handler cause/reason with the refusal appended. Once the caller receives
Rejected, it replaces its report with Failed(the returned failure), drops original effects permanently and retries
only that failed report. If a decision already is a no-effects terminal failure and the store refuses it, return
OpError::Invalid instead of another fallback, preventing recursion (including a wrong message id).

This costs a second round trip after known rejection, but keeps knowledge of the transition at the caller. An
internal server fallback would lose that knowledge if its response were lost and a peer retried the original effects.
A lost rejection response is different: no failure commit was attempted, so the original report can be retried and
re-evaluated against current store state, just as a local caller can retry an unacknowledged store result.

Cancelled means the worker's cancellation beat handler end; a late observed cancellation is ignored as today. The
server trusts this decision and cannot prove a handler stopped. It never releases a lease. A bad report or a refused
terminal failure is logged/abandoned by the worker without release, not turned into a handler retry.

## Uncertainty and verification

A repeated completion after application returns LeaseLost, not a replayed receipt. The same answer also covers
replacement/deletion, so it is not proof of success. Drop worker observers unless Completed is acknowledged. GET of
the message cannot resolve all uncertainty: retention may hide its outcome, or the commit may have rescheduled it.
External effects remain at least once. No receipt table, migration or Backend behavior change is needed.

Tests cover every report/effect/cause, Pending and Running retry snapshots, state/stop checks, identity derivation,
peer/stopped/unstarted servers, atomic rejection, and expiry fencing. Fault tests must cover unknown primary commits,
lost rejection responses, and unknown failed-report commits after an acknowledged Rejected; the latter must never
restore the original effects even if the absent target appears. Include retention-hidden outcomes and an assertion
that complete_turn never calls release. Drop the completion future during an in-flight commit, retry through a peer,
and verify at most one effect set/outcome, including SQLite's transaction-settlement boundary. Keep local runtime/fuzz
tests; update guarantee references and ADR 0038. Public docs distinguish deterministic report validation (may commit
a derived failed outcome immediately) from a backend refusal (Rejected, no fallback committed in that call).

The independent design review is ready for tasks; its cancellation-test and public-documentation notes are included.
