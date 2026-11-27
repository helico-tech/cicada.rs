Verdict: fix needed

## Blocking

1. `crates/cicada-client/src/changes.rs:161-170`: a slow consumer gets a false `Idle`. The idle deadline counts from
   the last chunk the reader *pulled*, and `select!` is `biased` toward the timer. If the caller spends 45 s or more
   between two `next()` calls (for example, relisting a large prefix after `Resync`, which the example tells it to
   do), the next poll returns `Idle` at once, even when keep-alives or hints are already waiting in the body. Design
   decision 3 says `Idle` means "no byte at all has arrived for 45 s". How to see it: a virtual-time test that yields
   a `Resync`, then `advance(46 s)` with the next chunk ready (`stream::iter` of two frames). The second `next()`
   returns `Err(Idle)`. I ran this in a scratch worktree and it failed. Fixed: poll the body before the timer (put the
   chunk branch first in the biased `select!`, or check for a ready chunk before the deadline), and add that test.

## Notes

1. Only lines are capped. The data buffer of one event grows with no limit across many `data:` lines that never get
   an empty line. Consider capping the event too, or say in the docs that only lines are capped.
2. `futures-core`/`futures-util` are pinned directly in `cicada-client/Cargo.toml`. Every other crate takes `futures`
   from the workspace. `futures-util` is not in the design (only a private dependency, so not a break).
3. `Transport` throws away the reqwest error, so callers cannot tell a read timeout from a reset. A source with the
   URL removed would keep the text free of secrets.
4. A non-200 body that is not a problem gives no `detail`. Other calls keep the body text there. This is deliberate,
   for secrets, and matches the handoff. Fine, just a difference.

Checked and fine: ADR 0077, the ADR 0035 status line, the index row; the 24 h 1 min per-request timeout; the 10 s
`Body` timeout; `Api` with the HTTP-status fallback when the body is late or not a problem (the body text is not
echoed); the content-type check; parser rules (BOM, CR/LF/CRLF splits, the space after the colon, a line with no
colon, comments, unknown events, an incomplete final event, 64 KiB); `Debug` removes user, password, query, fragment
and headers; `#[non_exhaustive]`; CHANGELOG; the handoff targets exist; no protected docs; lines ≤120. Every spec
scenario has a test. I ran `cicada-client`'s change tests (unit, `tests/changes.rs`, doctest): all pass.

Not checked: the workspace suite, clippy, the gate.
