# 0061. Retry in the job graph

Date: 2026-11-24

## Status

Accepted for job-graph-retry. Supplements ADR 0059 and ADR 0060, and refines ADR 0059's "a report target treats a
repeat for the same run as one" into "the highest end number per run wins".

## Context

After ADR 0060, a failed or cancelled job stays so, its dependents stay skipped, and an ended run stays ended. CI
pipelines, media processing and imports retry one job without running the whole graph again; brood's pipeline reopens
a failed or cancelled job and its skipped dependents. A run that ends twice sends two different end reports, which a
report target cannot order by arrival: a rescheduled turn changes the order of its mailbox (G5).

## Decision

- `Retry` puts one failed or cancelled job back to `waiting`, and the normal start runs in its commit: the job gets the
  next attempt number there when its needs succeeded and a slot is free, else later. Every skipped job becomes `waiting`
  and the skips are computed again from the current statuses. An ended run goes back to `running`.
- A retry is refused for good, the state unchanged, for an unknown job, a job that is not failed or cancelled, and a
  job with a need that is failed, skipped or cancelled, and when an attempt number or the count of ends would pass
  its largest value. There is no cap on retries; the kept attempt records stay bounded.
- The run counts its ends, and the end report carries the number of its end. A report target keeps, per run, the
  report with the highest number; one with the same number is a repeat. An end reopened before its report went out
  is never reported, so numbers can have gaps.
- A cancel notice reaches a replaced attempt's worker whatever the job's status is by then, so a retry right after a
  cancel does not leave the old worker running.

## Consequences

- A result or failure notice of a replaced attempt is refused or ignored by the rules that already match the current
  attempt; a `Stopped` about a reused target still fails the new attempt.
- The end report gains a field in the same release as the job graph, so no host sees its contract change.
- A job waiting or cancelled again may keep old attempts, so the decoder allows a closed last attempt on those.
- A host that retries automatically writes its own policy and its own cap.
