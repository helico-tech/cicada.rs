Verdict: not yet

Blocking
1. Where errors surface is open (design Decision 4, spec). `changes` returns `ClientResult<ChangeStream>`, so a refused handshake would be a `ClientError`, yet `Handshake` and `Status` are `ChangeStreamError` variants. Also "Status with the problem's text" drops the `Problem` that `ClientError::Api` keeps, so a caller cannot tell 401 from 404. And a 200 that is not `text/event-stream` (a proxy page or a wrong base URL) would end as `None`, which reads as "store closed". Fix: after `changes().await`, 10 s timeout -> `Handshake`, non-200 -> `Api(Problem)`, wrong media type -> `Body`; only body errors go in the stream. Add a spec scenario for each, and for malformed data and a clean end.
2. ADR 0035's Consequences say "No new cicada-client streaming API ... or dependency". Name that and lift it, by a short new ADR or a line the owner agrees to.

Notes
- Checked reqwest 0.13.5: a per-request `timeout()` does replace the client's total timeout for the whole body. 24 h is safe, since tokio caps a long sleep. The client's `read_timeout` still applies, and its `is_timeout` would look like `Lifetime`. Run the 24 h deadline on the stream's own Tokio timer, which the crafted-stream test needs anyway, or drop `Lifetime`.
- Parser: drop one space after the colon (`event: changed`), and treat a line with no colon as a field. Cap the line buffer (-> `Malformed`).
- `futures-core` becomes a public dependency. It is already in the tree. Use `Response::chunk()`, because reqwest's `stream` feature is off.
- 2-minute short-timeout test: virtual time over real TCP can be flaky.
- Handoff: architecture.md line 646.

Not checked: a build, the WHATWG text itself.
