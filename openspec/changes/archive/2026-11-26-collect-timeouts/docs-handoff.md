# Docs handoff for collect-timeouts

Version: 0.23.1. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` only; no contract, wire format, schema or HTTP change.

- `CopyLimits` (still `#[non_exhaustive]`) gains the public field `max_read`: the most content bytes of regular files
  one `copy_out` reads, kept and skipped alike. `CopyLimits::new(max_files, max_bytes)` sets it to 4 × `max_bytes`
  (saturating; `new` is now a `const fn`), and `CopyLimits::with_max_read(bytes)` changes it. Zero is `InvalidSpec`,
  as the other limits are. A regular file that would take the bytes read past `max_read` stops the copy before its
  data: the files before it are kept, and one last `SkippedFile` for the path says
  `the rest was not read: over the read limit of N bytes`. Docker (the archive stream is dropped there) and the fake
  stop at the same entry. `Outputs::new` gives 1000 files, 64 MiB kept and 256 MiB read.
- A collecting step of the container actor whose poll message failed 3 times (a turn past `MAX_TURN`, or any other
  failed try; the store counts them in the message's `failures`) makes no copy, log read or progress read on its next
  try, and records and sends the pending end with `outputs.error` =
  `(no outputs: the collection failed 3 times; last: <the message's last error>)` and `stored`/`bytes` counted from
  the files the store holds (0 when the store cannot list them, with a warning).
- Warnings: while collecting, a warning when the errors in a row reach `max_errors` and the end goes out without its
  outputs (`debug` for each retry before); while deleting stored outputs, one warning when the errors reach
  `max_errors`, and the deletion keeps trying (`debug` for the others).
- Behaviour change: a huge file early in an output folder now ends the copy there, so smaller files after it are no
  longer kept.

Proved by: `copy.rs` tests `the_read_limit_counts_skipped_files_and_stops_at_the_file_past_it`,
`the_read_limit_is_four_times_the_bytes_kept_by_default` and the zero case of `a_zero_limit_or_a_bad_path_is_invalid`;
`fake.rs` `copy_out_stops_in_a_file_over_the_read_limit`; `docker/archive.rs`
`an_entry_past_the_read_limit_ends_the_stream_before_its_data`; `tests/outputs.rs`
`a_copy_that_always_times_out_ends_after_three_tries`; `tests/warnings.rs` `a_store_that_keeps_failing_logs_one_warning`;
the rustdoc example of `ContainerJob::outputs` (asserts `max_read` of 256 MiB).

## 1. The read limit of `copy_out`

**Where.** `docs/guide/containers.md`, `Copy files out`, the paragraph that starts `CopyLimits::new(max_files,
max_bytes) bounds memory`.
**Text.** Add after the `max_files` sentence: `CopyLimits` also bounds what a copy reads: `max_read`, by default four
times `max_bytes`, counts the content bytes of every regular file, kept or skipped (Docker sends a skipped file's data
all the same). A file that would take the count past it stops the copy before its data, with one last `SkippedFile`
for `path` ("the rest was not read: over the read limit of N bytes"); the files before it are kept. Change it with
`CopyLimits::new(1000, 64 << 20).with_max_read(1 << 30)`. Also change "and the copy goes on" so it is clear that a
skipped file still counts against `max_read`, and add a zero `max_read` to the `InvalidSpec` list.
**Caveats.** From 0.23.1. A folder with a huge file before small ones keeps only the files before it, where 0.23.0
kept the small ones too. The order of entries is Docker's, so which files come before the huge one may differ between
engine versions.

## 2. A collection that keeps failing still ends

**Where.** `docs/guide/containers.md`, `Outputs at the end`, the bullets **Errors.** and **Turn limit and memory.**;
and the `MAX_TURN` bullet of `Define a job`.
**Text.** **Errors.** gains: a collecting turn past the job's `MAX_TURN` commits nothing and does not count as a
runtime error, but after 3 failed tries of the same poll, timeouts included, the next try makes no copy and sends the
end with `error: "(no outputs: the collection failed 3 times; last: …)"` and the counts of what the store already
holds. So a copy that always takes too long still ends, after about three turn limits. **Turn limit and memory.**:
"set the job's `MAX_TURN` above the time a copy of the read limit (`max_read`, 4 × the byte limit by default) takes".
The `MAX_TURN` bullet may say that the collection's copy is a third long turn.
**Caveats.** The 3 tries are counted per poll message: a log drain that commits arms a fresh poll, whose count starts
at 0. The last progress is not read on this path.

## 3. Warnings when the store keeps failing

**Where.** `docs/guide/containers.md`, `Outputs at the end` **Errors.**, and `Retention` **Errors.**; the
`max_errors` row of the settings table in `Host and start` may mention them.
**Text.** Collecting: at `max_errors` errors in a row the actor logs a warning as the end goes out without its
outputs. Retention: a store that keeps failing is tried again without limit; when the errors in a row reach
`max_errors`, the actor logs one warning (the retries log at `debug`).
**Caveats.** One warning per run of errors in a row; the actor's error count is in its state, so a restart does not warn again.

## Blocking

None.

## Status

`cargo test -p cicada-containers --all-features` passes; the finishing checks are listed in the builder's report. The
text above is proposed; no protected prose is written.
