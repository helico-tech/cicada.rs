# Design

Two holes in the collection at the end (`container-output-collect`), found by its code review and by Factory's design
review: a copy that always runs past the job's `MAX_TURN` never ends, and one big file over the limits is read in full
on
every try. Both are closed without a contract change, so this is additive (0.23.1).

## Decisions

1. **Timeouts are counted from the step message.** A turn that runs past `MAX_TURN` ends `failed` (`timeout`) and
   commits nothing, but the store counts it in the message's `failures`, which the next turn reads with
   `turn.message().failures`. A collection turn whose step message has `failures >= 3` does no long call and no
   progress read (it records the pending end directly, not through the finishing path that reads progress) and sends
   it with `outputs.error` = `(no outputs: the collection failed 3 times; last: <the message's last error>)` and the
   counts of what is stored; `failures` counts every failed try (a timeout, or a store error in the turn), so the text
   names the last error instead of assuming a timeout. So a copy that always takes too long still ends after three
   tries, before the
   step's retry policy (7 attempts) gives up and leaves the container waiting for a `Check`. Not taken: a state counter
   written before the long call (an extra commit per try and a state field, a contract change); a smaller copy per turn
   (the copy is one archive stream).
2. **A cap on what `copy_out` reads, skipped data included.** `CopyLimits` (`#[non_exhaustive]`) gains `max_read`, the
   most content bytes of regular files `copy_out` reads in total, kept and skipped alike (counted the same way by Docker
   and the fake, which has no archive); `CopyLimits::new` sets it to 4 × `max_bytes` (saturating), and
   `with_max_read(bytes)` changes it (zero refused like the other limits). Once the bytes read reach it, the copy stops
   reading, keeps what it has, and adds one skipped entry for the path: `the rest was not read: over the read limit of
   N bytes`, as the entry for `max_files` does today. Docker and the fake both stop there (the shared collector of
   `copy.rs`). `Outputs` takes its limits from `CopyLimits`, so the collection gets the cap. A change of behaviour for
   existing callers: a big skipped file before small ones now ends the copy there, where today the small ones are
   kept; the changelog says so under Changed. The rustdoc of `ContainerJob::MAX_TURN` ("waits for the next Check")
   names the three-tries rule. Factory asked for a default of a few times `max_bytes`; 4× it is. Not taken: a time
   limit alone (a big file is still read on every try).
3. **Warnings where a store keeps failing.** From the code review of `container-output-retention`: store failures
   while deleting outputs, and while collecting, log a warning once the errors in a row reach `max_errors` (they log
   at `debug` before), and the rustdoc of `max_errors` says the deletion keeps trying.

## Tests

On virtual time with a test runtime whose `copy_out` sleeps past `MAX_TURN`: after three timed-out tries the end is
sent with the error and the stored counts, and no fourth copy is tried. `copy_out` on the fake with a file larger than
`max_read`: the copy stops, keeps the files before it, and lists the rest as one skipped entry; `with_max_read(0)` is
refused; the default is 4 × `max_bytes`. A store that fails `max_errors` times logs one warning.

## Docs

The handoff gives the guide the read cap (with `max_bytes`), the three-tries rule and the warnings.
