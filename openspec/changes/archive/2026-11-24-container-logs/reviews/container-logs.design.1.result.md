Verdict: not yet

## Blocking

1. **The cursor assumes a `since` filter Docker may not apply.** design.md Decision 1 ("gives every line at or after it"); docker spec delta. In moby master (`loggerutils/logfile.go`, `forwarder.Do`), the `since` check stops at the first message at or after `since`. The stream then includes later lines stamped before `S`. Example: A (second T), B (stamped T-1, written after A), C (T). `next` counts 2 lines (A, C); `since=T` gives A, B, C, skips 2 and returns C again: a repeat. Older engines may filter each message. Fix: the pager drops lines stamped before `S` itself, so both behaviours, and the fake, give the same stream. Correct the sentence, keep the "missed late line" limit, add a pager unit test.

## Notes

1. Decision 1: "how many lines of this read, up to it" must include the `n` skipped lines. Say so.
2. proposal.md Impact contradicts Decision 5. Edit it.
3. bollard's `since` is `i32`: it overflows in 2038. Name the failure in the rustdoc.
4. Each poll makes Docker scan the current and uncompressed rotated log files from the start. Put the cost of fast polling in the handoff.
5. Lines over 16 KiB come back as several `LogLine`s. State it in the spec and rustdoc.

## Questions for the owner

None.

## Not checked

`write_log_stream.go` returned 404, so these are unverified: one frame per message, the timestamp format, newlines, partial-line framing, and how the API parses `since`. I read master only, not released engines. Confirmed in bollard 0.21.1: frames decode one by one; `ContainerSpec` has no TTY. Nothing built.
