---
id: actor-change-sse.code.3
kind: code
change: actor-change-sse
round: 3
base: 9d174a8a82b949a44887db76dd9f31f35b99cd97
head: 8f0c9fe6e1246e15d45d6cd7a24c2befaead3308
previous: 7435585c7430ca60f8320fd9b884419d47506848
requested: 2026-11-13T17:31:09+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-change-sse.code.3 <file>`.

## Note from the requester

Version: 0.16.11

Builder report (round 3: fix of round 2; the owner allowed this extra round)

The open part of finding 3 (disposal between a read's completion and its continuation) is fixed in 8f0c9fe on feat/actor-change-sse.

The JavaScript (docs/guide/http.md)
- A new stillLive() throws once the page is disposed. It runs right after every await, before anything is applied to the view or rendered, in seven places: in getJson after the fetch and after response.json(); in list after getJson returns; in read after the fetch and after response.json(); in work after list returns and after read returns.
- read was restructured so the 404 path and the JSON path both apply only after a check.
- The text above the example says a result arriving just before disposal changes and shows nothing.
- The example is only syntax-checked with node --check; not run (as agreed).

The Rust copy (crates/cicada-axum/tests/changes.rs)
- A check at each of those seven points and no other guard; the old extra check at the former line 548 is gone.
- At each check a test hook runs first, then the page's handlers (a disposal among them), then the check. The only other disposal checks mirror the JavaScript: the work loop's condition and "no retry after disposal".
- Existing tests hook "after a listing page" through the same check points.

New tests (replacing the earlier dispose-during-a-listing test)
- Listing: dispose at each of its four checks: the source is closed, no timer is left, the arrived page is not applied, nothing is rendered, and no request is sent later.
- Address read: dispose at each of its three checks, for a changed and a deleted actor: after the fetch or after response.json() the view stays as it was and nothing is rendered.
- Note: disposing after read itself has finished leaves the view changed but renders nothing; read updates the view before its promise settles, so that change came before the disposal. The test asserts exactly that.

Checks
- cargo test -p cicada-axum: 167 passed, 0 failed (20 in tests/changes.rs).
- cargo test -p cicada-examples --doc: 49 passed.
- cargo test -p cicada-examples --test docs: 11 passed.
- cargo fmt --all --check and cargo clippy --workspace --all-targets -- -D warnings: clean.
- Not rerun this round: cargo test --workspace (only tests/changes.rs and docs/guide/http.md changed since the last full run, 2456 passed).
