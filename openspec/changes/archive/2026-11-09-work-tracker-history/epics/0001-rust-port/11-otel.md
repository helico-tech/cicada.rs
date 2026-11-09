---
status: done
created: 2026-10-07
---

# 11. OpenTelemetry plugin

## Goal

OpenTelemetry plugin.

## Acceptance criteria

- Spans and metrics as `ActorTelemetry.kt`; its tests ported.

## Plan

Port `actors-otel` to `crates/cicada-otel` on the `opentelemetry` 0.33 API only (the SDK is a dev-dependency):

- `ActorTelemetry`, an `impl Plugin` named `ActorTelemetry`: `global()` (global tracer, meter and propagator, also
  `Default`), `new(&tracer_provider, &meter_provider)`, `with_propagator(p)`. Scope `nl.helico.cicada`.
- `on_claim` starts the `CONSUMER` span `process <template>` with the Kotlin attributes, its parent extracted from the
  message headers (case-insensitive names), and records `cicada.message.lag`; the span's context, start and
  injected headers live in the turn's `Attributes`. `on_handler_error` adds an `exception` event. `on_outcome` sets
  `cicada.outcome`, `cicada.sent`, the error status, ends the span (starting one when `on_claim` never ran) and
  records `cicada.turn.duration`.
- `trace_headers(&turn)` and `current_trace_headers()` for explicit propagation, as in Kotlin (the headers are plain
  `traceparent`/`tracestate`, so no reserved header and no core change is needed).
- Tests with `InMemorySpanExporter`, `InMemoryMetricExporter` and `TraceContextPropagator`.

## Progress log

- 2026-10-07: Ported; the 7 Kotlin tests pass. No change to `crates/cicada` was needed.
- The Rust W3C propagator writes an empty `tracestate`; empty header values are left out, so the headers equal
  Kotlin's (`traceparent` only).
- Deviation: the SDK for Rust ends and exports a span when it is dropped, so the span of a turn whose lease was lost is
  exported (without `cicada.outcome`), where Kotlin never exports it. Documented in the crate docs; see issue 0600.

## Follow-ups

- [0600 Spans of abandoned turns are exported](../../issues/0600-otel-abandoned-turn-spans.md)
