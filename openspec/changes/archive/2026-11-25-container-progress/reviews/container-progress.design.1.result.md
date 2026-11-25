Verdict: not yet

## Blocking

1. **"Checked at registration" cannot be built** (design 2, ADR). `Actor` has no registration hook, and `ContainerActor::new` cannot fail. Decide where `PROGRESS_FILE` is checked. My pick: at create, only with `progressType`, ending `failed`/`notStarted` the way `check_inputs` does. Also add to the rustdoc that a read-only root with a tmpfs `/tmp` gives no progress, because `copy_out` cannot see a tmpfs.
2. **A NUL drops progress without a word** (design 3, 5). Lossy UTF-8 keeps NULs. `send_raw` then refuses the send (`TurnError::Invalid`, limits.rs), and it counts as a "refused send". `text.rs` already sets the rule: replace NUL with U+FFFD where the text is read. Apply it here. Also say whether `seq` and the hash move on a refused send.
3. **"Progress arrives before the end" is false under faults** (design 3, spec scenario 2). If the owner fails to handle a progress message, the retry puts it back after the backoff. The end, due now, then overtakes it (architecture "Handler returns `Err`" row; G5). Reword: "sent in the end's commit, before it". Tell owners that a retried progress message can come after the end; they can skip it by `seq`. Also say which ends read the file: exited, runtime-failed, stopped and timed out, but not missing, notStarted or runtimeErrors.

## Notes

- Name the hash (e.g. SHA-256 hex; `sha2` stops being optional), the new state fields, and where `progressType` is kept (`EndTarget`, the first `Start` wins). Check `progressType` like `messageType`, so a bad one is refused for good.
- Rust source break: the new fields of `Start`, `EndTarget` and `ContainerState` break struct literals and patterns (e.g. tests/actor.rs:410). Put this in the upgrade handoff.
- The first read of an empty file: is it sent?
- Set the env var after the job's `spec`, so the job cannot override it.
- Pairing: once nothing is ready or in build, status.py says `release now`, so nothing holds the batch for 0.23.0. And container-output-collect's proposal has not yet decided whether the end report changes. Keep the release pairing out of the ADR.

## Questions for the owner

- Q1: Hold 0.23.0 for container-output-collect even if its design turns out additive? My pick: no.

## Not checked

How Docker's archive behaves on tmpfs (from Docker's docs, not tried). The container-log-archive worktree. No builds or tests run.
