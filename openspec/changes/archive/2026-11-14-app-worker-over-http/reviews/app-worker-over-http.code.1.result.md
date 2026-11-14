Verdict: fix needed

Blocking findings:

1. crates/cicada-app/src/config.rs:165–166: Target's new Debug exposes valid URL passwords containing an apostrophe. For server `https://user:can'ary@actors.internal/api`, hide_passwords stops scanning the authority at the apostrophe and returns the entire password unchanged. A standalone reproduction using the existing helper confirmed this. This violates the agreed credential-redaction behavior. Sanitize the URL structurally (with a conservative fallback for invalid input), or omit it from Debug, and add a regression covering this password.

2. crates/cicada-app/tests/worker_http.rs:254–285, 298–304, 393–395: these tests make runtime timing assertions on real time: an 800 ms handler must renew a 1 s lease, authentication failure must finish within 10 s, and a 200 ms sleep is used to assert shutdown is still draining. This contradicts AGENTS.md and the design's restriction of real sockets to framing checks without timing claims; the builder's stated departure does not describe these assertions. Move timing/stop assertions to paused-time tests with controlled I/O barriers (the predecessor has a harness), retaining framing/auth assertions on real sockets. The shutdown test currently can also pass its waiting assertion before shutdown was processed.

Notes: worker_client leaves reqwest's default protocol-NACK retry policy enabled. Explicit retry::never() would make the documented no-client-retry behavior robust when a consuming application enables reqwest's HTTP/2 feature through Cargo feature unification.

Not checked: no suite, build, clippy, formatter, feature checks, or Docker E2E run. Builder-reported results were reviewed as reported. Inspected the commit diff, agreed artifacts, existing Worker coverage, and local reqwest retry implementation; only the standalone redaction experiment was executed.
