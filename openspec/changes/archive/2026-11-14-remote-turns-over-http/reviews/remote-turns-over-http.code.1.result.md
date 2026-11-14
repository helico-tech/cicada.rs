Verdict: fix needed

Blocking findings

1. `crates/cicada/src/api/leases.rs:157` (also the nested effect records at lines 179–238): derived struct deserializers accept positional arrays, despite the agreed object-only wire shapes and rejection of mistyped fields. A valid completion envelope with `"effects":[]` decodes as empty effects; `"effects":{"controls":[["delete","c/d"]]}` decodes as a delete. The route's `unique_body` checks only the outer object, then uses this decoder, so these malformed reports reach core completion instead of returning 400. Confirmed with `/tmp/cicada-review-arrays.rs`, linked against the builder's artifacts; creates and watches also accept arrays. Require object forms for envelope records (while retaining arbitrary JSON state/payload), and add API/route regressions asserting 400 and no operation for positional records.

2. `crates/cicada-client/tests/remote_worker.rs:389` (also line 240 and the fault-test sleeps): the deadline test sleeps 3.5 real seconds before asserting abandonment; the peer test sleeps four real seconds to prove renewals. These are runtime timing assertions, not socket-framing smoke tests. They violate AGENTS.md and task 4/spec “Two-server and fault verification,” which explicitly require virtual time with controlled I/O barriers. Replace wall-clock race/deadline assertions with the agreed controlled virtual-time harness; retain real-socket smoke coverage separately if useful.

Notes

- `docs/guide/http.md:319`: distinguish a body-read transport error (Unavailable) from a successfully read but malformed JSON body (Fatal).
- All six tasks are checked and the version is 0.16.16.

Not checked: no suite rerun, release checks, or Docker E2E. Builder-reported fmt, clippy, workspace/PostgreSQL, feature and OpenSpec results were assessed as reported. Reviewed implementation, tests, generated-reference changes and agreed scenarios; ran only the focused decoder experiment above.
