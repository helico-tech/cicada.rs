Verdict: pass

## Blocking findings

None. Tasks 1–6 are ticked and their work is in the diff. Every spec scenario has a test: 27 in `tests/permits.rs`, and the state bounds are unit tests in `src/permits.rs`. The fencing, the FIFO grant loop, the self-only `Notify`, the store-written `Failed` filter (`actor`, `from` and `notification`), the freeing of an undelivered grant and decision 10's `notified` guard in both `Audit` and `notify` all match the design. The version is 0.20.8 in `Cargo.toml`, both skills, `plugin.json` and `openapi.json`. The changelog, the contract reference and the rustdoc table are updated. No protected docs were edited. The handoff is true to the code and calls nothing written.

## Notes

1. `src/permits.rs:371`: one rustdoc line is 179 columns, after the decision-10 edit. Rewrap it to 120.
2. `undelivered` matches a request ID, not a grant number. A late `Failed` could free a newer, unnotified grant if the same ID is used again. This is not reachable while IDs are fresh, as decision 4 requires. Carrying `grant` in `Notify` would close it.
3. When `nextGrant` reaches `u64::MAX`, `grant_waiting` refuses for good any message that would grant, `Release` included, so holders could no longer release. This cannot happen in practice. A sentence in the rustdoc would make it explicit.
4. Handoff "Behaviour": "A NUL is refused by the core at the send". The handler's `check_message_type` refuses it first. Small wording point.
5. `CHANGELOG.md`: the `Audit` sentence runs to 169 columns. Rewrap it.

## Not checked

I ran no builds, tests or gate; I relied on the builder's gate evidence for 9bd00e2. I did not diff `cicada.permits.json` line by line against the schemas (`wire_formats` checks it). I did not check E2E, which runs at the release. ADR 0058 was already on main and was not reviewed here.
