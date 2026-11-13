# 0032. Timers start linked traces

Date: 2026-11-13

## Status

Accepted for `otel-timer-traces`. The owner delegated design decisions to an Opus 5.5 adviser.

## Context

ActorTelemetry puts outgoing messages in the sending turn's trace. A self-rearming timer therefore creates one
ever-growing trace, including every tick's work. The stored envelope no longer says whether the sender requested a
delay. The core knows that at send time, but the observer currently adds identical headers to every send.

## Decision

- Extend ADR 0018's header-only observer facility with declarative selection: all sends, timers, or others. Existing
  add_send_header still reaches all sends. A timer targets the current actor and requests Delivery::After or At,
  including zero delays and past times. Classification does not depend on the commit clock or the actor's type.
- cicada-otel puts its configured propagator's headers on ordinary sends, and the same headers prefixed with
  `cicada-link-` on timers. This prefix is a stable persisted convention of cicada-otel, not a core-reserved header.
- A receiving turn extracts its parent from plain headers and at most one link from the prefixed headers, using the
  configured propagator and empty contexts. A valid link is supplied before span creation. No plain parent means a
  new root; a link is never promoted to parent. Explicit parent headers remain effective alongside the link.
- Core header precedence, validation, capacity and panic isolation are unchanged. Link extraction recognizes the
  prefix without regard to case, with exact-name lookup before case-insensitive lookup. Missing or invalid context
  omits the link. Neither link headers nor their omission affect actor behaviour or message acceptance.
- Flow sleeps and deadline self-messages use this rule; immediate flow continuations and delayed sends to other
  actors retain parent propagation. No type-specific exception, per-send callback or core marker is added.

## Consequences

- Each timer retry attempt is normally a new root linked to the original sending turn; operator retry copies the
  same headers. Trace ids are not derived from message ids. Hosts can still supply an explicit parent.
- Old queued timers retain their parent until rearmed by upgraded telemetry. Old receivers ignore the link prefix
  and start roots without links. Different configured propagators can lose links, as they can already lose parents.
- Header limits can discard link fields. Links are best effort. Root sampling follows the host's sampler, which may
  differ from parent-based sampling before this change; no new sampling policy or baggage propagation is added.
- No changes to G1–G6, S1–S10, store schema, HTTP fields or actor contracts. Core remains independent of OpenTelemetry.
  The architecture and guides are updated when built to describe the observable change and the header convention.
