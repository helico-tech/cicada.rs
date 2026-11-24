# Proposal

## Why

After `job-graph-cancel`, a failed or cancelled job stays failed and its dependents stay skipped. CI pipelines, media
processing and imports need to retry one job without running the whole graph again. brood's pipeline retries a failed
or cancelled job under a new run ID. Issue #2 (higher-level actors) adds that a late result from an attempt that a
retry replaced must be refused.

## What Changes

- `Retry` of one failed or cancelled job starts a new attempt of it, and puts its skipped dependents back to waiting.
- Each attempt has its own number per job (from `job-graph`); a result from a replaced attempt is refused for good and
  recorded as a failed message of the run, never taken, and a failure notice about a replaced attempt's job message is
  ignored (`job-graph-failures` matches a notice to the current attempt only).
- The run that had ended starts again, and reports a new end with a higher end number.

The decisions are in [design.md](design.md) (ADR 0061).

## Out of scope

Automatic retry policies per job (a host can send `Retry` from its own policy). A graph that changes while it runs.

## Order

After `job-graph-cancel`, before `container-consistency`.

## Impact

The coordinator of `job-graph` and its tests and contract; the end report gains its end number. Additive: 0.20.8,
the unreleased batch, shipped with `job-graph`, `job-graph-failures` and `job-graph-cancel`. An ADR. The docs handoff
extends the guide section of `job-graph`.
