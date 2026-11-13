---
id: actor-change-sse.code.2
kind: code
change: actor-change-sse
round: 2
base: 9d174a8a82b949a44887db76dd9f31f35b99cd97
head: 7435585c7430ca60f8320fd9b884419d47506848
previous: e7135a3f7679e90ff0219e12f2cfd0d87ece1853
requested: 2026-11-13T17:23:28+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-change-sse.code.2 <file>`.

## Note from the requester

Version: 0.16.11

Builder report (round 2: fix of round 1)

All three blocking findings of round 1 are fixed in 7435585 on feat/actor-change-sse.

1. A closed stream showed "live" again (docs/guide/http.md): the guide's JavaScript keeps the connection state (connecting, live, reconnecting, stopped) apart from read success; every render shows that state, so once CLOSED a finished listing or read no longer shows "live". The work loop also stops once disposed. The Rust copy of the consumer does the same; new tests: CLOSED during a listing (the listing finishes, shown stays [Stopped, Stopped]); CLOSED during a delayed retry (retry succeeds, stays stopped); error while CONNECTING (reconnects, the new stream's first resync lists again, a write made while disconnected is picked up, shown [Live, Reconnecting, Live]).

2. Dropped subscription not checked (crates/cicada-axum/src/changes.rs): a new unit test feeds the response from a channel standing in for the core's subscription; it drops one idle response (only the keep-alive timer armed) and one active response (a hint unread); in both cases the channel reports closed, so the source was released. It also checks the number of live tokio tasks is unchanged across the drop. The integration test says where the release is checked.

3. No disposal coverage (crates/cicada-axum/tests/changes.rs): the Rust consumer has dispose() and one retry timer mirroring the guide's later(). Two new tests: dispose during a listing (source closed, the page in flight unused, no timer left, no request after a write and a wait); dispose during a delayed retry (timer cleared, retry never runs after ten retry periods).

The JavaScript stays guide-only (syntax-checked with node --check); the Rust copy is what the tests run. AtomicUsize::try_update in a test helper replaced by a compare-and-swap (needs a newer Rust than the minimum 1.94).

Checks
- cargo test -p cicada-axum: 166 passed, 0 failed (19 in tests/changes.rs, 3 in the changes unit tests).
- cargo test -p cicada-examples --doc: 49 passed.
- cargo test -p cicada-examples --test docs: 11 passed.
- cargo fmt --all --check and cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2456 passed, 0 failed.

No new departures or breaking changes.
