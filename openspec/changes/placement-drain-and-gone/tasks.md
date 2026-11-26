# Tasks

Normal lane, additive: in the unreleased 0.24.0 batch (check the `release:` line of `python3 scripts/status.py`,
builder.md, Version). Builds on `placement-asks` and `placement-holders` (land first). Follow `design.md`; the spec is
`specs/placement-pool/spec.md`. The code is `crates/cicada-actors/src/placement.rs`.

- [x] 1. **Drain.** `Drain` and `Undrain` with checked constructors; the node field `draining`; a draining node counts
  as
  no free places while the never-meet check still counts it; offers keep the flag; undrain scans the line; repeats
  change nothing (decision 1). Verify: the drain tests of the design.
- [x] 2. **Gone.** `Gone` refused for good for a live or unknown node; for a `silent` node: removed, places freed, one
  self `TellGone { holder, grant, node, to, messageType }` per holder in its own turn sending a `gone` answer;
  `TellGone` from anyone else refused for good; the line scanned (decision 2). Verify: the gone tests of the design.
- [x] 3. **Contract, changelog and docs handoff.** The answer kind `gone`, the new messages and field in the contract
  reference and samples (`CICADA_UPDATE_CONTRACTS=true`); the 0.24.0 section names drain and gone under Added; the
  handoff carries the design's Docs section and the warning about a `gone` for an unseen grant. No edit to
  `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-actors`,
  `cargo test -p cicada-examples --test docs` and `--doc`.
- [x] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-drain-and-gone --strict`, commit, then
  `python3 scripts/gate.py run` (in the background with a 2-hour timeout).
