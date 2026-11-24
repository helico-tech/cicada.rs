Verdict: pass

## Blocking findings

None.

All five tasks are ticked, and their work is in the diff. 0.21.0 is set in `Cargo.toml`, `Cargo.lock`, both skills, `plugin.json` and `openapi.json`; no 0.20.9 is left outside the change. The changelog heading is renamed, and its summary covers the whole batch and names the break. Every `failed` path in `actor/mod.rs` gets its kind from the table of decision 3 through `fail`/`refuse`. The adoption skips `spec`, `inputs`, the copies and `start`. `RawEnd` decodes old ends without `kind`, and refuses a kind on other ends in both the state and `ContainerEnded`. Every spec scenario has a test. The contract is regenerated. No protected docs are edited, and the handoff matches the code.

## Notes

1. `CHANGELOG.md` "Changed", the refused-start entry (and design decision 5), describe 0.20.8 wrongly. At v0.20.8 a start error left the phase `New`, so each poll called `create` and `start` again. It ended after `max_errors` with "the runtime failed N times in a row", not "at a later poll with Docker's reason".
2. The same entry says a start refused with `Other` "still counts as an error", which reads as unchanged. On Docker a failed start sets `State.Error`, so `status_of` gives `Failed`. The next poll now adopts that container and ends it `runtimeEnded`, where 0.20.8 retried `start`. A passing 500 on start now ends the job. The design accepts this limit, but the changelog should call it a change.
3. `RawEnd` repeats the fields of `ContainerEnd`, while the schema comes from the struct. A field added to only one would drift; a comment would help.

## Not checked

I ran no tests, gate or E2E. I did not check Docker's `State.Error` on a failed start against a real daemon.
