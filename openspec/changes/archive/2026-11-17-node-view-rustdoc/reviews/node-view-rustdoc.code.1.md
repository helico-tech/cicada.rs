---
id: node-view-rustdoc.code.1
kind: code
change: node-view-rustdoc
round: 1
base: 8b649b4d3ae17a8f4b367935728c1f465c7b9c00
head: ffad77447a5b578cabeb4781b05370a59f547c00
requested: 2026-11-17T19:16:55+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done node-view-rustdoc.code.1 <file>`.

## Note from the requester

Version: 0.20.5 (patch: one public doc comment; no behaviour, signature, wire, contract or schema change).

Head: feat/node-view-rustdoc at ffad774, based on main 8b649b4 (which holds this change's tasks.md; not yet on origin).

Builder report:
- 43bbd3c: ActorSystem::nodes doc in crates/cicada/src/runtime/system.rs now reads "For observability, including node-down alerts. The node table grants no authority to claim, move or fence actors; leases decide who runs what." The first sentence (node table, status from store time, running turns) is unchanged. Version 0.20.5 in Cargo.toml, Cargo.lock, docs/openapi.json (version line only), .claude-plugin/plugin.json and both skills; CHANGELOG entry; docs-handoff.md (no guide change; asks the build loop to correct the node-view spec Purpose's "display only" wording at archive time).
- ffad774: ticks task 2.
- Checks: cargo doc -p cicada --no-deps, cargo fmt --check, --test docs (11), --test skill (18), openspec validate --strict, OpenAPI test.
- Gate: full checks and E2E (31 passed) on 43bbd3c; docs run (34 doctests, 11 docs, 18 skill) on ffad774. gate.py status: "checks: yes, docs pass over the full pass of 43bbd3c7009d", "e2e: yes, e2e pass of the full-checked base 43bbd3c7009d".

Left out on purpose (the design limits the change to ActorSystem::nodes): other comments still say "display only": crates/cicada/src/api/mod.rs:387 (public NodeInfo doc), runtime/results.rs:117, store/model.rs:511, store/mod.rs:313, store/sqlite/schema.rs:47, and ADR 0015. The build loop will raise them with the owner separately; they are not part of this change's scope.
