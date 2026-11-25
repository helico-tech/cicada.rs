Verdict: pass

## Blocking findings

None.

## Notes

1. CHANGELOG.md: the branch base (72e11e87) has no 0.21.2 section, while main (1c5f9860) has `## 0.21.2 — unreleased` (docker-context). At rebase, merge that section into `## 0.22.0 — unreleased` and widen the summary to the whole batch, as tasks.md says. The requester already knows.
2. Departure: each store's commit refuses `existing` on a type, on `*`, or without `set` (`support::check_watches`, `StoreError::Invalid`). The design does not describe this. It is defensive, matches `check_controls`, has a conformance case, and the changelog names it. Fine, but ADR 0063 and the design do not mention it.
3. postgres/store.rs, commit: the absent `Stopped` notices are added after `notices()`, so they skip its "removed or gone watcher" filter and depend on `if !stopping`. This is correct today, because `check_controls` forbids a commit that deletes its own actor and an ancestor delete is rejected. But it is shaped differently from SQLite and the in-memory store, which pass `absent` into `notices()`. A comment, or passing them in the same way, would keep the backends aligned.
4. crates/cicada-client/tests/remote_worker.rs: rustfmt reflowed the unrelated `Hold`, `Forward` and `Big` variants onto several lines. This is harmless noise in the diff.
5. docs-handoff.md is complete and true to the code at head. I checked the absent `Stopped` fields, the order and dedup rules (`support::notifications` sorts and dedups by about, watcher and type), the stopping watcher, the 400 at conversion, and the fixture names.

Checked: every task is in the diff, and every spec scenario has a test (conformance, the pool runtime tests, the leases unit test, the axum 400 test, the remote worker test). The PostgreSQL lock covers the delete side: `delete` re-reads the watches after locking and retries (`TxError::Again`) on a watcher it has not locked. Also checked: version 0.22.0 in Cargo.toml, both skills and plugin.json; openapi regenerated; ADR 0063 and its index row are in the base; public items have doc comments; no protected docs edited.

## Not checked

I ran no tests and did not verify the gate evidence for 6305fcfb. I did not confirm that the race test fails when the target lock is removed.
