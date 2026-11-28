---
id: container-network-isolation.code.1
kind: code
change: container-network-isolation
round: 1
base: ebc09060f6ef88368754d6a0f1e867e0583f2038
head: 8d985dbc506974e84e3b35e5d7d4970e0e143746
built-by: gpt-6.2-sol
requested: 2026-11-28T12:12:37+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-network-isolation.code.1 <file>`.

## Note from the requester

Version: 0.26.0

Builder's report (gpt-6.2-sol), head 8d985dbc506974e84e3b35e5d7d4970e0e143746 on feat/container-network-isolation:
- Tasks 1–5 done; implementation follows the agreed design. Additive Rust API change; custom runtimes must honour `isolated` or return `Unsupported`.
- Example: `let mut network = NetworkSpec::new("net"); network.internal = true; network.isolated = true;` passed to `ensure_network` makes an isolated internal bridge on Docker 28+; older negotiated APIs return `Unsupported`; either flag differing on an existing network returns `Conflict`.
- Checks: gate passed 4,191 workspace tests and 63 Docker E2E scenarios, 0 failures; `cargo test -p cicada-containers --all-features` 323 passed; fmt, strict clippy and openspec validate passed. The last commit passed the docs gate over the full and E2E pass of 6049f3e1a9a7.
- Docs: handed off, no blockers (docs-handoff.md).
- Departures and open questions: none reported.
