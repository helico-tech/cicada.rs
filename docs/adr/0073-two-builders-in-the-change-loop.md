# 0073. Two builders in the change loop

Date: 2026-11-26

## Status

Accepted for change-loop-two-builders. Supersedes the "one builder at a time" rule of ADR 0055; the rest of ADR 0055
stands.

## Context

Builds and gate runs are the slow part of the change loop. The line holds work that touches different crates (the
placement pool, and the new tray and inspector crates), so one builder at a time leaves one kind waiting behind the
other. The owner wants a second builder on another provider's model and every change reviewed by a model other than its
builder's.

## Decision

- Two builders may run at once when their changes' `Crates:` lines share no crate; shared workspace files are merged at
  landing. The first runs as a Claude subagent, the second on Codex (`gpt-6.2-sol`, effort `high`) in its own worktree.
- A change is reviewed by the other model; `scripts/review.py` records the builder and refuses a same-family reviewer.
- The gate keeps one run at a time. One session describes, lands and releases, one change at a time.
- When Codex is unavailable, the second change is built by Claude and its review waits for GPT.

## Consequences

Two lines of work move at once, and every change gets a second model's eyes. Landing gets more rebases and ADR-number
checks. A third builder or a second session would need a new ADR.
