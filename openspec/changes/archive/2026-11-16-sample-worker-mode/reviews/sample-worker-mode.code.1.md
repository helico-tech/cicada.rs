---
id: sample-worker-mode.code.1
kind: code
change: sample-worker-mode
round: 1
base: 523580b180244843bc353f80ce24013d87e3763f
head: 46fa9d93d63e31956c707118553927202c56d621
requested: 2026-11-16T23:47:14+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done sample-worker-mode.code.1 <file>`.

## Note from the requester

Version: 0.20.3

Builder report (feat/sample-worker-mode, head 46fa9d93d63e; rebased by the build loop onto main 523580b before the docs integration, clean, main had only docs commits):

Tasks: all six done.
1. Identity: the sample's test actors and effect log record Turn::node_id (the actual claim owner); the old node-id arguments are kept but ignored.
2. Separate executable: Sample::executor and the cicada-sample-executor binary share a main; default-run keeps cargo run on the original binary.
3. HTTP acceptance.
4. Container: both binaries in the image; new E2E class SampleExecutorTest (host port 9095, because 9092 was taken on this machine by another project's Kafka).
5. Handoff and release; the design's condition "verify the docs session's prose has landed before landing" is met by integrating the owner's prose (below).
6. Finishing.

Checks: cargo test --workspace with PostgreSQL 3117 passed (first run); clippy -D warnings and fmt clean; --doc 34, --test docs 11, --test skill 18; openspec validate --strict valid. Gate on 5750e18 (integrated prose): full pass and E2E 31 passed. Final head: "checks: yes, docs pass over the full pass of 5750e18e9a5d", "e2e: yes, e2e pass of the full-checked base 5750e18e9a5d", docs record pass.

Behaviour changes in the sample (please judge against the design): a --node-id that differs from CICADA_NODE_ID now starts the sample and wins (before, it stopped the start); the old node-id arguments of the actor and Catalog constructors no longer label records. No compile-time break; state/message shapes and contract hashes unchanged.

New concept: the sample executor, the sample's types and routes without its on_start bootstrap (no singletons, no dead-letter watch), so it can run as worker --server; a node of the original sample or an API caller creates what a workload needs.

Docs: owner fixes integrated (ADR 0045, task 5). Docs commit 202790559aa7b5a2185dcc169afbf9d7758ff8be (parent 523580b), whole files docs/guide/app.md, docs/operations.md, e2e/README.md, taken with git checkout, no merge, committed in 5750e18 with the task-5 tick. The build loop checked: the commit holds only these three; they equal 2027905 and the docs branch on the branch head. Please check the prose describes the code. No prose left to write.

Breaking: none for library users.
