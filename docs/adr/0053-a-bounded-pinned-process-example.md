# 0053. A bounded pinned-process example

Date: 2026-11-16

## Status

Accepted for pinned-process-example after Opus 5.5 review and resolution of its findings, split from pinned-actors under the owner's autonomous design delegation.
Application example only; extends no core guarantee, API or store schema. Uses ADR 0052's placement semantics.

## Context

A process handle cannot live in durable actor state. Spawn and stop are outside effects that can precede a lost
turn commit. A stored PID does not establish ownership after restart, and a fresh host cannot infer whether an old
spawn happened. A complete production supervisor/reconciliation service is larger than a worked example.

## Decision

- Use one fresh never-reused actor key per process run and one fresh epoch per host-process lifetime. Pin the actor
  directly to the configured stable node id; runtime placement and host epoch have different meanings.
- Store desired/observed state and epoch in the actor. Retain live handles and terminal tombstones in at most 32
  host slots. One owned task starts each slot at most once and reaps its child. Terminal slots are never reused.
- Treat repeated Start as the same run, and Stop as sticky, including before spawn. Version observations and send
  them through a bounded host pump; retries/lost commits reuse the slot, not a new OS action.
- On a new host epoch, mark old runs NeedsReconciliation. Do not adopt/signal a saved PID or automatically respawn.
  NeedsReconciliation is absorbing, including late original-host observations. After external verification/cleanup,
  the operator may delete that actor and explicitly create a replacement with a new key. A dead host can leave orphans.
- Use only the example executable's inert child mode; no shell, arbitrary commands or descendants. Explicitly wait
  for exit after a stop request. Bounded shutdown reports unconfirmed cleanup instead of claiming success.
- Test actor/runtime races with virtual time and a fake process backend; check real OS plumbing separately with a
  bounded subprocess smoke test. Documentation distinguishes these evidence levels and the host-lifetime limit.

## Consequences

This teaches durable intent versus live resource ownership and repeat handling without promising exactly-once OS
effects or portable crash cleanup. It cannot restart a completed run or supervise arbitrary programs. Existing
ready-made supervisor/flow/event protocols cannot create pins and are not extended. Host shutdown and explicit
Stop, rather than deleting actor state, own process cleanup. The docs session receives the guidance through handoff.
