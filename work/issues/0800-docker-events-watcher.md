---
status: todo
type: idea
created: 2026-10-11
---

# 0800. Docker events watcher for container handles

## Problem

Container handles see a change only at the next `Check` (every 15 s by default), and every handle costs one turn per interval. Thousands of handles, or a need for fast reactions to an exit, outgrow polling (ADR 0007, Consequences).

## Notes

- A task per node (not an actor) that follows `docker events` filtered on the label `cicada.uri` and sends `Check` to that URI on `start`, `die`, `oom` and `destroy`.
- Several nodes may send the same `Check`: give it an id derived from the event (container id + event time) so the send is deduplicated (S7).
- Keep polling as the safety net, at a longer interval.
