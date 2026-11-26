Verdict: pass

## Blocking findings

None.

## Notes

1. Scope: the collecting warning (`actor/mod.rs`, `collect`, the `Halt::Retry` arm) also fires when runtime errors reach `max_errors`, not only store errors as decision 3 says. The builder names this departure, and the handoff describes it in neutral words ("the errors in a row"). It is harmless, but the design could say so when the change is archived.
2. `give_up` calls `store.files` with no limit of its own. If that call hangs past `MAX_TURN`, each later try times out again, and the container waits for a `Check` as it did before. This is unlikely, and the design accepts the read. A one-line caveat in the handoff would do.
3. The give-up count is per poll message. A `Check`, `Start` or `Stop` during the retry backoff arms a fresh poll that starts at 0. The handoff caveat covers log drains but not these messages; it could name them.
4. `Collector::offer` also stops at a file that would be kept (`max_read` < `max_bytes`). That matches the spec ("kept and skipped alike"). The rustdoc of `with_max_read` could say that a value below `max_bytes` is allowed and is the tighter limit.

## Checked

- Tasks 1-4 are ticked and their work is in the diff. Task 5 is unticked on purpose. The gate has a full pass recorded for 30b76d4b (`gate.py status 30b76d4b`).
- Both spec scenarios have tests: `a_copy_that_always_times_out_ends_after_three_tries` (begun == 3, error, counts) for the actor scenario; `copy_out_stops_in_a_file_over_the_read_limit` and `an_entry_past_the_read_limit_ends_the_stream_before_its_data` for the runtime scenario.
- `CopyLimits` stays `#[non_exhaustive]`, so the new field is additive. `new` as a `const fn` is additive too.
- A fresh poll resets the failure count, so a poll that failed in an earlier phase cannot trigger a give-up. The deletion warning fires once per run of errors (`errors` is reset when the container is removed).
- The rustdoc of `MAX_TURN`, `max_errors`, `copy_out` and `CopyLimits` is updated. The CHANGELOG has entries under both Added and Changed. The version is 0.23.1 in Cargo.toml, both skills and plugin.json.
- No protected docs changed. The handoff is true to the code, Blocking is none, and its status says no prose is written.

## Not checked

I ran no tests myself and relied on the gate record. I did not check the Docker side against a real daemon (no E2E run).
