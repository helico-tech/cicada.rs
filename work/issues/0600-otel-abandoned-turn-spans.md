---
status: todo
type: question
created: 2026-10-07
---

# 0600. Spans of abandoned turns are exported

## Problem

`ActorTelemetry` in Kotlin never ends the span of a turn whose lease was lost before the commit, so that span is not
exported. The OpenTelemetry SDK for Rust ends and exports a span when it is dropped, so in `cicada-otel` such a span is
exported when the turn's attributes are dropped: without `cicada.outcome` and `cicada.sent`, status unset.

## How to reproduce

Install `ActorTelemetry` with an SDK tracer provider, let a turn lose its lease (a store that refuses renewals, or a
handler that outlives the lease TTL), and look at the exported spans: one has no `cicada.outcome`.

## Notes

- Options: (a) accept and document it (done in the crate docs); (b) mark it, e.g. `cicada.outcome` = `abandoned`, from
  a `Drop` guard in the plugin's turn attribute; (c) build the span only in `on_outcome` with a recorded start time
  and pre-generated span/trace ids, which complicates sampling. (a) is the current state; (b) is cheap if dashboards
  need to tell abandoned turns apart.
