Verdict: pass

## Blocking findings

None.

Tasks 1-5 are ticked and their work is in the diff. Each spec scenario has a test: "A week after removal", "No retention", "A store that is down at the due time", "A store that refuses the name" and "A lost retention poll" in `tests/outputs.rs`. The existing "A purged poll" test is unchanged. The design's other tests are there too: zero and capped retention, a restart with the retention unset, no store at the due time, a 0.22 state and the field outside `removed` in `wire.rs`, and the contract snapshot.

The code matches the design:
- `work_left` is true in `Removed` while `outputsUntil` is set.
- `remove` takes the container out of the runtime before it saves the due time.
- `expire` retries `Unavailable` and `Other` with the backoff, with no limit, and clears the due time on `Invalid` or when there is no store.
- A poll before the due time re-arms for it (the departure the builder named; `Check` needs it when no `due` is saved).

The versions are 0.23.0 everywhere. `CHANGELOG.md` covers the setting under Added and the state field under BREAKING. No protected docs are touched. The handoff is accurate, and its test names exist.

## Notes

1. `expire` logs retryable store failures only at `debug`. A store that is down for days stays silent at the default log level. Consider a `warn` once `errors` reaches `max_errors`, as other paths escalate.
2. The rustdoc of `ContainerSettings::max_errors` says "A removal keeps trying". It could also say that a retention deletion keeps trying.
3. No test checks that the warning is emitted, although the spec says "cleared with a warning". The behaviour around it is tested.
4. `Start`, `Stop` and `Remove` sent to a removed container whose outputs wait now re-arm the retention poll through `rearm`. This is harmless and matches "a goal asked again re-arms", but neither the handoff nor the rustdoc says so.

## Not checked

- I ran no tests and relied on the builder's gate evidence for d2414784.
- I did not check how the retention poll behaves against the PostgreSQL or SQLite stores at `MAX_INSTANT` (only the in-memory test covers it).
- I did not check the archiver race at zero retention beyond reading that a stale cursor after `remove` is refused.
