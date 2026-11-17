---
id: sample-heartbeat-watch.code.1
kind: code
change: sample-heartbeat-watch
round: 1
base: a1cff63063faa6244d605ef648dc3487e71810a2
head: d87ee2b1fd0e5d113dc3cd32fa2e9c2279e61afd
requested: 2026-11-17T08:54:45+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done sample-heartbeat-watch.code.1 <file>`.

## Note from the requester

Version: 0.20.4 (patch: only the sample service changes; libraries, actor contracts, wire formats, schema and guarantees are those of 0.20.3).

Head: feat/sample-heartbeat-watch at d87ee2b, rebased onto main a1cff63 (main only changed docs/how-to/install.md since the branch base).

What the builder did (tasks 1 and 2 ticked; 3 and 4 open):
- sample-service/src/catalog.rs hosts the existing HeartbeatWatchActor as cicada.heartbeat-watch in the shared catalog. Both binaries and the image register it under --types all or by exact name; --types none leaves it out. No singleton, Beat, Expire or startup hook.
- Contract evidence: sample-service/tests/contracts/cicada.heartbeat-watch.json and its digest in sample-service/tests/contract-texts.json; the snapshot's parsed JSON equals docs/reference/contracts/cicada.heartbeat-watch.json; earlier sample snapshots unchanged.
- Tests in sample-service/tests/sample_service.rs: both_definitions_host_the_heartbeat_watch_by_all_and_by_name_not_by_none_and_start_no_watch, a_caller_starts_a_heartbeat_round_on_the_sample (direct store), a_caller_starts_a_heartbeat_round_through_the_executor_over_http (send-only server and executor worker in process). Before Beat: waiting, no armed, empty mailbox; after: alive, lastBeatAt set, one pending Expire whose id is armed.
- Version pins 0.20.4 in Cargo.toml, Cargo.lock, docs/openapi.json, .claude-plugin/plugin.json and both skills; CHANGELOG entry; docs-handoff.md proposes one App guide paragraph (not blocking for doctests/links/routes, but task 3 wants the owner's prose integrated before landing).

Builder-reported runs: cargo test -p cicada-sample-service (all pass), cargo test -p cicada-actors --test wire_formats (22 passed), the two named virtual-time heartbeat_watch tests (2 passed).

Gate: the builder's full gate run was interrupted (the builder session ended). The build loop is rerunning `gate.py run --e2e` on d87ee2b now; I will report its result before landing. The owner's App prose will arrive as a Markdown-only commit on top.
