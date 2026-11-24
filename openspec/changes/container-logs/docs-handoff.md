# Docs handoff for container-logs

Version: 0.20.9. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` can read a container's log in pages, so a host follows a run's output while it runs and keeps it
outside the container. Additive; no README or guide block, link, guarantee row or skill route names the container
runtime.

- `ContainerRuntime::logs_since(name, cursor: Option<LogCursor>, max_lines: u32) -> Result<LogPage, ContainerError>`.
  `LogPage { lines: Vec<LogLine>, next: LogCursor }`, `LogLine { time: Timestamp, stream: LogStream, text: String }`,
  `LogStream::{Out, Err}`, `LOG_PAGE_MAX` (1000). All exported from the crate root; `LogPage` and `LogLine` are
  `non_exhaustive` with `new` constructors.
- `LogCursor`: opaque; `Display`/`FromStr` and serde as a string, JSON schema of a string. A bad string is
  `InvalidSpec`.
- The method has a default that fails with `Other`: a host's own runtime compiles unchanged.
- `FakeRuntime::push_log(name, LogLine)` adds a timed line (times may repeat or go back). `set_logs(name, text)` now
  gives one `Out` line per text line at the Unix epoch; `logs` and `logs_since` read the same lines.
  `Call::LogsSince { name, cursor, max_lines }` and `Method::LogsSince` record and script the call.

Proved by the unit tests in `crates/cicada-containers/src/logs.rs` (the pager and the cursor), `src/fake.rs`
(`scripted_log_lines_are_paged_and_read_by_logs`, `set_logs_gives_out_lines_at_the_epoch_to_both_calls`,
`logs_since_of_a_missing_or_created_container_and_a_bad_page_size`, `a_scripted_failure_of_logs_since_is_recorded`,
`a_runtime_without_logs_since_compiles_and_fails_with_other`) and `src/docker/map.rs` (the frame parser and the
2038 check), and by `ContainerDockerTest` in `e2e/tests/suite/container_docker.rs`
(`log_pages_give_each_line_once_with_its_stream`, `a_follower_collects_every_line_of_a_running_container_once`,
`log_pages_of_what_has_no_lines_and_bad_page_sizes`, `a_log_byte_that_is_not_utf8_is_replaced`). The rustdoc of
`logs_since`, `LogCursor` and `DockerRuntime` says the same.

## 1. Following a container's log

**Where.** The container guide, when the docs session writes it (no guide covers `cicada-containers` yet): the
runtime section, after `logs`.
**Text.** `logs(name, tail_lines)` gives the last lines as one string, for a short report. To follow a run while it
goes on, call `logs_since` with the cursor of the page before (`None` the first time). Each page holds up to
`max_lines` lines (1 to `LOG_PAGE_MAX`, 1000), each with its time, its stream (`Out` or `Err`) and its text as lossy
UTF-8 without the newline, and `next`, the cursor to call again with. Paging to the end gives every line once, in log
order, also when lines share a second. A page with fewer than `max_lines` lines has read everything there is now:
wait a few seconds and call again with `next`; an empty page keeps the cursor. Keep the cursor as a string (its
`Display`, or serde) in the host's own table, next to the lines it stored, so a restarted follower goes on where it
stopped. Call it outside actor turns: a page is too big for actor state.

A follower, as the E2E step `a_follower_collects_every_line_of_a_running_container_once` runs it (not compiled from
the guide; make it a doctest of `examples/` if it goes in):

```rust,ignore
let mut next: Option<LogCursor> = load_cursor(run)?; // None the first time
loop {
    let page = runtime.logs_since(&name, next, 500).await?;
    store_lines(run, &page.lines, page.next)?; // the lines and the cursor in one write
    next = Some(page.next);
    if page.lines.len() < 500 {
        if ended(runtime.inspect(&name).await?) { break } // after a short page of an ended container: done
        tokio::time::sleep(Duration::from_secs(3)).await; // poll gently
    }
}
```

Errors: a missing container is `Conflict` (also when it is removed during the read), a created one gives an empty
page, `max_lines` out of bounds or a string that is not a cursor is `InvalidSpec`, and a log Docker cannot read (log
driver `none`) is `Other`, so a follower stops retrying. Only `DaemonUnavailable` is worth a retry.
**Caveats.** The cursor's form is private: never build or parse it, only keep and give it back. It stays readable
across 0.20 patches and means the same on the fake and on Docker. On Docker: each read scans the log from the cursor's
second, so a tight loop over a busy log is costly (poll every few seconds); a line longer than 16 KiB comes back as
several lines, as Docker stores it in parts; a line stamped before the cursor's second but written after it (a race
of microseconds between stdout and stderr at a second boundary) is missed; log rotation that removes part of that
second shifts the count; a cursor after January 2038 fails with `Other`, as Docker's `since` is 32 bits.

## 2. Timed lines in the fake

**Where.** The container guide, the section on testing with `FakeRuntime`.
**Text.** `set_logs(name, text)` gives a container its output as `Out` lines stamped at the Unix epoch; `push_log(name,
LogLine::new(time, stream, text))` adds one timed line, with times that may repeat or go back, as on Docker. `logs`
and `logs_since` read the same lines, and the fake pages exactly as the Docker backend does. `fail_next(Method::LogsSince,
kind)` scripts a failure; `calls()` records `Call::LogsSince`.
**Caveats.** `set_logs` replaces every earlier line, the pushed ones too.

## 3. A runtime of the host's own

**Where.** The container guide, where it says a host can implement `ContainerRuntime`.
**Text.** `logs_since` has a default that fails with `Other` ("not supported by this runtime"), so a runtime written
before 0.20.9 compiles unchanged; implement it to let followers page its logs.

## Blocking

None.

## Status

`cargo test -p cicada-containers --all-features` (84 unit, 22 and 6 integration tests) and `cargo test -p
cicada-examples --test docs` pass; `ContainerDockerTest` passed against Docker (19 steps). All text above is proposed;
no guide covers `cicada-containers` yet, so nothing is written in `docs/`.
