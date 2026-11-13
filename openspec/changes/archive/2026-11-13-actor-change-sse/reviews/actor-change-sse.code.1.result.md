Verdict: fix needed

1. docs/guide/http.md:338,354,362: a permanent EventSource failure is hidden by an outstanding refresh. Start a listing, deliver CLOSED/error, then complete the listing successfully: the example renders “stopped” followed by “live”, although no stream will reconnect. Reproduced with the exact guide function and mocked EventSource/fetch in Node. Keep connection status independently of read success and preserve the stopped indication. Add coverage for CLOSED during a read/retry and CONNECTING recovery (the “Browser reconnect and authentication” scenario).

2. crates/cicada-axum/tests/changes.rs:261: “Client disconnect” checks only that other requests survive; it never checks that the dropped subscription is released. Leaking both dropped bodies would still pass. Add an observable ownership/drop assertion for idle and active bodies; an adapter unit test with a drop-probed source can do this without exposing the core's private receiver count. Verify dropping the response releases its source and pending keep-alive work.

3. crates/cicada-axum/tests/changes.rs:324 and docs/guide/http.md:399: the “Encoding and disposal” scenario lacks disposal coverage. The Rust consumer has no disposal operation or retry cancellation, so the encoding test cannot establish that pending fetch/retry work is cancelled. Add focused lifecycle coverage (Rust model or JavaScript harness) that disposes during a read and during delayed retry and verifies source closure and no subsequent work.

Notes: No rejecting input was identified for the existing lenient string-pair query parser; its error propagation remains intact. The future-variant fallback shares the tested resync arm. All tasks are ticked; version, framing, OpenAPI and shutdown ordering match the design.

Not checked: suite reruns, browser/proxy execution or Docker E2E; relied on the builder's reported checks.
