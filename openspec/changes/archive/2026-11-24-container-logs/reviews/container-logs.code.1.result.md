Verdict: fix needed

## Blocking

1. `openspec/changes/container-logs/docs-handoff.md`, section 1, the follower example. It says it runs "as the E2E step `a_follower_collects_every_line_of_a_running_container_once` runs it", but it does not. It checks `inspect` after a short page and then breaks. If the container writes its last lines and exits between that read and the `inspect`, those lines are never read. The E2E step avoids this: it checks the status first, reads again, and stops only on a short page read after the exit was seen. Fixed means the example checks `ended` before a read and breaks only on a short page from a read that started after the end was seen (as the E2E step does), or the handoff names this caveat.

## Notes

1. Doc comments over 120 columns: `fake.rs` (the `FakeRuntime` doc, the `push_log` line), `runtime.rs` (the `LOG_PAGE_MAX` line of `logs_since`), and the module doc of `e2e/tests/suite/container_docker.rs`. rustfmt does not wrap comments.
2. The handoff Status says "84 unit" tests, but the builder report says 112 for `--all-features`. Make the counts agree.

## Not checked

I ran no tests, builds or E2E; I relied on the builder's gate evidence for `bbfdfef`. I did not check against Docker that a frame of a non-TTY read is always one whole entry, or that log driver `none` gives 501, which maps to `Other`. I did not check what bollard's TTY newline split does to a split 16 KiB line (the spec has no TTY option). I checked the pager logic, the cursor, the fake, the Docker read, the scenario coverage, the version pins, CHANGELOG, and that no protected docs are in the diff.
