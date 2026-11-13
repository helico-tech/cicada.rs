# Proposal

## Why

`ActorTelemetry` puts every message a turn sends in the turn's trace. That is right for a chain of work, but wrong for
a timer: an actor that re-arms itself with a delayed message builds one trace that never ends. Each tick is one span
deeper, and everything a tick sends lands in the same trace. The trace only ends when a node restarts.

Found in cicada-code-factory: its reconciler sends itself a `Tick` every two minutes. In mallard the trace grew one
level per tick, and every ticket sync of every tick joined it. The same holds for any actor that polls or re-arms
(a cron chain as in `examples/src/cron.rs`, a poll loop, and probably the scheduler's slots).

A host can work around it today: a send that sets its own `traceparent` keeps it, so the code factory sends the W3C
invalid all-zero `traceparent` on its tick, which starts a new trace. Every host would have to do this, and the link
to the turn that set the timer is lost.

## What Changes

- The turn of a timer starts a new trace, with a span link to the turn that sent it. A timer is a message an actor
  sends itself with a later delivery (`SendOptions::delay` or `at`).
- Other sends stay in the sender's trace, as now. This includes immediate self-sends, such as a flow's `Continue`, so
  a flow's steps stay one trace.
- The observer cannot tell a timer from other sends today: `TurnStart::add_send_header` adds the same headers to every
  send, and a `Message` holds `deliver_at` but not whether a delay was asked. The simplest way out is for the design
  to settle. Options: the core tells the observer, per send, whether it is delayed, so `cicada-otel` writes a link
  header in place of `traceparent`; or the core marks a delayed send with a header that the receiving turn reads.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `turn-observer`: an observer can tell a delayed send from an immediate one (or the equivalent the design picks).
  The trace rules of `cicada-otel` follow from it.

## Out of scope

Spans or logs from handler code; sampling; delayed sends to other actors (such as the rate limiter's), which stay in
the sender's trace; a sleep inside a flow (the design should say whether it counts as a timer); the HTTP API; any
change to how immediate sends carry their trace.

## Order

After `flow-events`, the last of the line.

## Impact

The observer API in `crates/cicada` (per-send information about delays, or a marker on delayed sends), the trace
links in `crates/cicada-otel` and its tests on virtual time (`crates/cicada-otel/tests/actor_telemetry.rs`), the
section on observers and OpenTelemetry in `docs/guide/core.md`, and the changelog. Perhaps an ADR if the observer's
contract changes (ADR 0018). Once it is released, cicada-code-factory drops its workaround (its
`telemetry::new_trace`).
