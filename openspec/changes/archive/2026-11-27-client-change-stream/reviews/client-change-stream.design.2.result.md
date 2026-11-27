Verdict: ready for tasks

Both blocking findings are fixed. Open errors now come from the call as ClientError (Api for non-200, Body for a wrong content type), with spec scenarios for refused, not an event stream, malformed data and a clean end. ADR 0077 partly supersedes ADR 0035's no-streaming line, and the handoff names the architecture.md line. All earlier notes are handled: Lifetime on the stream's own timer, read_timeout as Transport, one space after the colon, a line with no colon, the 64 KiB cap, the real-time test, and the secrets test.

Notes (not blocking)
- Decision 2 says the 10 s handshake timeout returns ClientError::Http. A reqwest::Error cannot be built by hand, and reqwest's per-request timeout covers the body too. So wrap send() in tokio::time::timeout and return ClientError::Invalid or Body("timed out waiting for response headers"). Change the wording so the builder does not guess.
- No spec scenario covers the 10 s handshake timeout. It is only in the tests list.

Not checked: a build, the WHATWG text.
