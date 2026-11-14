Verdict: fix needed

Blocking findings:
1. `.claude/skills/build-loop/SKILL.md:77–80` (also Land, lines 99–102): affected-check reruns are conditional on detecting a changed path or behavior and obtaining fresh preparation. Follow the clean-rebase path with identical owner-doc blobs: the loop can inspect prose, then immediately merge, without running any documentation checks against the rebased source/base. This misses the spec’s explicit “conflict-free late rebase ... affected-check verification” requirement; unchanged prose can still stop compiling against a new base or dependency configuration. Require the affected checks on the combined result after every rebase of a branch carrying owner fixes, before landing, even when no replacement prose is needed. Keep refresh conditional on invalidation, and record this unconditional check in the clean-rebase walkthrough.

Notes:
- The docs-session merge risk is adequately identified in this change’s handoff: prepare against the feature head, do not merge code-dependent prose into main first, and keep blocked work waiting until explicit coordination/adoption. No protected-guide edit is requested by this review.
- Ownership, exact-file integration, rename/deletion handling, blocked-build resumption, version/release exceptions, truthful handoff status and final owner approval otherwise align with the artifacts. Version fields consistently say 0.18.6.

Checked: commit range, change artifacts, applicable architecture/ADR and ownership/release instructions, ownership-hook behavior, reported checks, and workflow paths. The reported existing global OpenSpec failures are outside this diff.

Not checked: test suites or reported counts independently; live docs-owner coordination, CI, or release execution. No tests were rerun.
