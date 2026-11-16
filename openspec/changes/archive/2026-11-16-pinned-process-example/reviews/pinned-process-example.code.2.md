---
id: pinned-process-example.code.2
kind: code
change: pinned-process-example
round: 2
base: 52ac854c793f7d5572be23c74237f977f64d7d84
head: 56c082120b4bfd55e76d020f00f6ef7718bde30f
previous: fcba7e4499bf4a08fea1b062c50346e5b7baa957
requested: 2026-11-16T21:12:21+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done pinned-process-example.code.2 <file>`.

## Note from the requester

Version: 0.20.1

Round 2. Fixes for the round-1 blocking findings; head 56c082120b4b (previous head fcba7e4), one commit. Not rebased onto main yet.

Finding 1 (demo cleanup):
- New public `pinned_process::run_host(store, system, host, body)`: attaches the pump and runs the host's work; however it ends, shuts down in order (host cleanup within IO_TIMEOUT, bounded system stop, pump stop and join, store close) and returns the work's original result with the cleanup report. The report gains a `pump` field, set when the pump task fails.
- All the demo's work after system.start() runs inside run_host; the binary prints unconfirmed cleanup and returns the original error. If system.start() fails, the store is closed before returning the error.
- Test: `an_error_while_a_child_runs_still_cleans_up_in_order_and_is_returned`. By hand: an invalid --new-run key ran cleanup, reported the error, exited 1.

Finding 2 (check order), RunActor::handle now: 1) record Stop; 2) if the status is final (terminal or NeedsReconciliation), refuse Start (Stopped if a stop was asked, else Ended); 3) if node id or host epoch mismatch, set NeedsReconciliation, no process action; 4) only then refuse Start after Stop.
- Test: `a_start_on_an_old_epoch_marks_a_stopping_run_for_reconciliation_before_any_refusal` (fails with the old early refusal restored).

Checks on 56c0821: fmt, workspace clippy -D warnings clean; --test pinned_process 20/20; --test pinned_process_os 3/3 (macOS only); --test docs 11; --test skill 18; --doc 30; openspec validate --strict valid. gate.py status: full pass and e2e pass of 56c082120b4b.

Departure: run_host is a new public item in the example's library module, so the cleanup order can be tested; changelog and docs-handoff.md describe it and both new tests.

Breaking: none. Docs: handed off, no blockers.
