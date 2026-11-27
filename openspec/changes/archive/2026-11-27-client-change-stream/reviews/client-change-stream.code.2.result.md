Verdict: pass

## Round 1 findings

1. Fixed. `read_stream` spawns one reader task per stream. It ends when the stream is dropped: a biased
   `changes.closed()` branch, or a failed `send`, drops the `Response` and so the connection. The channel holds one
   hint and backpressure blocks the reader, so memory stays at one queued hint, one being sent and the rest of one
   chunk. One producer feeding a FIFO channel keeps the order. The reader resets idle as it reads, which is about when
   bytes arrive. After backpressure, a ready chunk beats the old deadline (the `select!` order is lifetime, chunk,
   idle). The 46 s slow-consumer test and the full-buffer test both pass.
2. Fixed. `MAX_EVENT_DATA` caps joined data at 64 KiB (newlines included), with a boundary test.
3. Fixed. `futures-core` comes from the workspace and `futures-util` is gone (Cargo.lock too).
4. Fixed. `Transport(reqwest::Error)` uses `without_url()`. The tests check `is_timeout()` for a read timeout and
   for a reset, and that no URL or secret shows.

## Notes

1. No test drops a stream and checks that the task ends and the connection closes. The code is right, but a test
   would guard it.
2. A false Idle is still possible in theory, when a byte is still in the kernel and hyper has not read it yet.
   hyper reads one chunk ahead, so this is unlikely.

## Not checked

The workspace suite, clippy, the gate. I ran `cargo test -p cicada-client` at 9f60789f: all pass.
