Verdict: ready for tasks

Blocking findings: none.

The change defines one bounded behaviour and fits the proposed task count. Its public Flush semantics explicitly cover early delivery, retries and redrive into a later burst. Cancellation, forwarding and clearing fit the existing atomic turn API; missing-target commit failures and registry-read retries preserve the pending value. State-depth accounting and failure causes agree with ADR 0029 and the executor. The preceding limiter and following batcher proposals introduce no conflict.

Notes:
- In the implementation tests, include a Submit and Flush with the same delivery timestamp in both sequence orders. The existing `(deliverAt, seq)` rule determines whether that submission replaces the current burst or starts the next; this needs no new policy.

Questions for the owner: none.

Checked the proposal, design and complete spec delta; architecture and relevant ADRs; existing scheduler, stored-message, timer, contract and sample-service code; core send/cancel/context/time handling; relevant existing specs and adjacent proposals. OpenSpec list/show/validate succeeded for this change.

Not checked: implementation correctness, generated contract output or runtime behaviour of the future actor. No test suite was run, and the preceding limiter's in-progress implementation was not reviewed.
