# Design

`logs(name, tail_lines)` gives the last lines as one string. A host that shows a run's output while it runs, and keeps
it outside the container, needs to read on from where it stopped, without repeats or gaps. The shape is agreed with
Factory, the consumer:

```rust
async fn logs_since(&self, name: &str, cursor: Option<LogCursor>, max_lines: u32) -> Result<LogPage, ContainerError>;
pub struct LogPage { pub lines: Vec<LogLine>, pub next: LogCursor }
pub struct LogLine { pub time: Timestamp, pub stream: LogStream, pub text: String }
pub enum LogStream { Out, Err }
```

`None` reads from the first line. A page with fewer than `max_lines` lines has read everything there is now; the host
calls again later with `next`. The text is lossy UTF-8 without the newline, as in `logs`. It is called outside actor
turns: a page is too big for actor state, and the actor does not use it.

## Decisions

1. **The cursor is a second and a count.** It holds `S`, a Unix second, and `n`, the number of lines of the stream
   from `S` that came before it. The stream from `S` is Docker's log read with `since=S`, from which the pager itself
   drops every line stamped before `S`. A read skips `n` lines of that stream and returns the next `max_lines`. `next`
   holds `S'`, the second of the last returned line, and the number of lines of this read up to and including it,
   the `n` skipped ones counted too, that are stamped at or after `S'`. Docker's `since` takes whole seconds
   (bollard's `since: i32`), so a time alone repeats or drops lines that share a second. The pager drops early lines
   itself because Docker's own filter differs by engine: moby master stops checking at the first line at or after
   `since`, so later lines stamped before it come through, while older engines filter each line (design review,
   blocking 1). Without the drop, lines A (second T), B (stamped T-1, written after A) and C (T) give a cursor of
   (T, 2) after C, and `since=T` then gives A, B, C: C comes again. With the drop, every engine and the fake give A, C.
   Counting positions, not times, also holds when lines are out of time order within a second: Docker stamps stdout
   and stderr in two copiers, so a later line can carry an earlier time. Not taken: the last line's time with
   nanoseconds and how many lines had that time (it skips a line that lands after the cursor with an earlier stamp in
   the same second); trusting Docker's filter (a repeat on current engines); `tail` (counts from the end).
2. **`LogCursor` is an opaque string.** `Display` and `FromStr` give and take it, and `Serialize`, `Deserialize` and
   `JsonSchema` treat it as a string, so a follower can keep it in a table or in actor state between calls. Its form
   (now `S:n`) is private and kept across 0.20 patches; a string that does not parse is `InvalidSpec`. Both runtimes
   use the same form, so a cursor is not tied to one. Not taken: public fields (callers would build cursors and fix
   the form); no serde (every follower would write its own).
3. **`max_lines` is 1 to 1000.** Outside that, `InvalidSpec` before any call to Docker; `LOG_PAGE_MAX` names the
   bound. Memory stays bounded: Docker splits a line at 16 KiB, so a page holds at most about 16 MiB. Not taken: no
   bound (one call can hold a whole log in memory); clamping (a caller that asked 5000 and got 1000 lines would think
   the log was read to the end).
4. **What a container gives.** A missing one fails with `Conflict`, as `logs` does, also when it is removed during the
   read. A created one gives an empty page; a running or ended one gives its lines. `next` of an empty page is the
   cursor given, or the start for `None`. Not taken: `Conflict` for a created container (a follower that starts
   before the program would have to wait for `Running` first).
5. **The trait gets a default method.** `logs_since` has a default that fails with `Other` ("not supported by this
   runtime"); the fake and Docker override it. Adding a required method to a public trait breaks every implementation
   outside this repo at a patch release, which Cargo treats as compatible. This supersedes the proposal's Impact line
   ("every implementation outside this repo must add it"). `container-copy-out` follows the same rule. Not taken: a
   required method (a break in a patch); a default built on `logs` (it has no times, so it cannot page).
6. **One pager for both runtimes.** A private pager takes the lines from `since=S` in log order, the cursor and
   `max_lines`, and gives the page; both runtimes call it, so the fake pages exactly as Docker does. The Docker
   backend reads with `timestamps=true` and `follow=false`, parses each line's RFC 3339 time (`cicada::time`, no new
   dependency) and drops the stream once the page is full. Each frame of Docker's stream is one log entry: a time, a
   space and the text; a frame that does not start with a time fails with `Other`. Not taken: paging in each backend
   (they would drift, as `copy_in` did).
7. **The fake scripts timed lines.** `push_log(name, LogLine)` adds a line; times may repeat or go back, as on Docker.
   `set_logs(name, text)` stays and gives one `Out` line per text line at the Unix epoch, so `logs` and `logs_since`
   read one store. `Call::LogsSince` and `Method::LogsSince` record and script the call (both enums are
   `non_exhaustive`). Not taken: a second store for timed lines (the two calls could disagree).

## Limits

The rustdoc of `logs_since` and `LogCursor` names each one; the handoff carries them to the guide.

- A line stamped before `S` but written after the cursor is missed, such as B above. It needs a race of
  microseconds at a second boundary between stdout and stderr.
- Each read makes Docker scan the log from second `S`, through the current and uncompressed rotated files from their
  start: one second with many lines makes every page in it cost more. A follower of a long run polls gently (every
  few seconds), not in a tight loop.
- Log rotation that removes part of second `S` shifts the count.
- A line longer than 16 KiB comes back as several `LogLine`s, as Docker stores it in parts.
- bollard takes `since` as an `i32`: a second after January 2038 does not fit, and `logs_since` fails with `Other`
  for such a cursor rather than read from the wrong time.

## Spec deltas

`container-runtime`: *Logs can be read in pages* is added; *A fake runtime for tests* is modified (timed lines).
`container-docker`: *Log pages come from Docker's timed stream* is added. No guarantee, wire format, schema or actor
contract changes, so no ADR. `LogLine` has no serde: the host stores lines in its own form.

## Tests

Unit tests of the pager (lines sharing a second, out of time order within a second, a page cut in a second, an empty
read, a repeated cursor, and lines A, B, C of decision 1, where B is dropped and C does not come again), of the cursor's
string form, of the frame parser (time, stream, lossy text, a missing time), and of the fake (`push_log`, `set_logs`,
scripted failures). The E2E class `e2e/tests/suite/container_docker.rs` adds: paging with `max_lines` 7 through 50 lines
on both streams in one second gives each line once, in order per stream, with its stream; a follower polling a container
that prints across several seconds collects every line once; the same cursor twice gives the same page; a missing
container is `Conflict`, `max_lines` 0 and 1001 are `InvalidSpec`, a created container gives an empty page; a byte that
is not UTF-8 becomes U+FFFD.

## Docs it breaks

None: no README or guide block, link, guarantee row or skill route names the container runtime. `docs-handoff.md`
carries the call, the cursor, the limits above (gentle polling, split long lines, 2038) and the follower loop for
the runtime section of the container guide.

## Version

0.20.9, with the rest of the container line; released when the line, through `container-limits-and-kinds`, has
landed.
