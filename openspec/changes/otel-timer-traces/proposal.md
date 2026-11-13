# Proposal

## Why

`ActorTelemetry` puts every message a turn sends in the turn's trace. That is right for a chain of work, but wrong for
a timer: an actor that re-arms itself with a delayed message builds one trace that never ends. Each tick is one span
deeper, and everything a tick sends lands in the same trace. Queued messages retain their headers, so a node restart
alone does not break the chain.

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
- Add `TurnStart::add_header_for(Sends::{All, Timers, Others}, name, value)`. The existing method still reaches all
  sends. Timer classification uses the requested delivery, including zero delays and past absolute times.
- On timers, `cicada-otel` writes its configured propagator's context under `cicada-link-` header names. Receivers
  extract a link from those headers and their parent from plain headers. Explicit parent headers remain effective.
- Flow sleeps count as timers; ordinary flow continuations do not. Each timer retry attempt gets its own root trace
  linked to the original sender. ADR 0032 records the persisted header convention and compatibility boundaries.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `turn-observer`: an observer selects headers for timer sends or other sends; `cicada-otel` links timer traces.

## Out of scope

Spans or logs from handler code; sampling policy changes; changing delayed sends to other actors (such as the rate
limiter's), which stay in the sender's trace; changing flow sleep behaviour beyond its trace boundary; the HTTP API;
any change to how immediate sends carry their trace; propagating baggage; grouping timer retries in one trace.

## Order

After `flow-events`. No following change.

## Impact

The observer API and send-header selection in `crates/cicada`, the trace links in `crates/cicada-otel` and its tests
on virtual time (`crates/cicada-otel/tests/actor_telemetry.rs`), the core and actors guides, architecture, ADR 0032,
and the changelog. No store schema, HTTP or actor contract change. After release, cicada-code-factory can drop its
workaround (`telemetry::new_trace`); that host change is outside this proposal.
