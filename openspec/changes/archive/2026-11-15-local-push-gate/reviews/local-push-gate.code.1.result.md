Verdict: fix needed

Blocking findings:
1. `scripts/gate.py:1114` (also 1016, 1064 and Git helpers): release subprocesses bypass `execute`, inheriting neither the lock nor a tracked process group. In a temporary release with a sleeping fake gh, killing the gate left gh alive, the marker's child null, and `recover` returned success and cleared the marker while gh still ran. A second publisher can then overlap an unfinished publication. Route release subprocesses through supervised execution with lock inheritance, group tracking and cancellation/reaping; regression-test kill/recovery and TERM during publication, not only check commands.
2. `scripts/gate.py:719–731`: the previous pass is invalidated only after checkout preparation, configuration validation and tool-version subprocesses. Reproduced both an external-config refusal and TERM during a blocking `rustc -V`: `run --force` failed/interrupted, but the full record remained `pass` and `checks_auth` still authorized it. Write unusable running evidence before fallible execution/preparation, mark normal failures failed, and test these early failure/interruption paths for every rerun kind.

Notes:
- The accepted-main rule and real partial-push regression address the design blocker. Policy coverage, conservative Markdown classification, protected-doc ownership and version 0.18.9 align with the change; the final commit changes only handoff/tasks Markdown. Strict OpenSpec validation passed.
- Draft upload is supported: GitHub CLI's [FetchRelease implementation](https://github.com/cli/cli/blob/trunk/pkg/cmd/release/shared/fetch.go) explicitly searches drafts; this is not a blocker.

Not checked: no broad suite rerun, Linux execution or real GitHub publication. Full/PostgreSQL/Docker results are the builder's reported checks. Targeted reproductions used temporary repositories and fake gh only; no shared hook, evidence or remote was changed. Reproduction scripts: `/tmp/cicada-gate-review-repro.py`, `/tmp/cicada-gate-review-force-interrupt.py`.
