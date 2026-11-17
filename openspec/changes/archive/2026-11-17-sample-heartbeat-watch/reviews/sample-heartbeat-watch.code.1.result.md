Verdict: fix needed

Reviewed a1cff63063faa6244d605ef648dc3487e71810a2..48df3a0facee8de5f07d9f128ef5cf0d6c127ee7 (the final head supplied by the owner, superseding the request's d87ee2b).

Blocking findings:
1. openspec/changes/sample-heartbeat-watch/docs-handoff.md:57 still says the owner has not prepared prose and task 3 remains open. At this head, a8beaa8 has integrated the App paragraph and ticked task 3. The handoff also omits the owner-prepared commit/path required by the review brief and ADR 0045. Replace the stale status with the actual integration and verification status; record owner commit 28519faffa82ef22d8f07fb022c3074df9c04b1e and docs/guide/app.md. I verified that file exactly matches the owner commit. Refresh docs gate evidence after this Markdown fix.

Notes: No implementation or scenario-coverage blockers found. Both definitions share the existing actor directly; selection and startup exclusion are covered, as are caller-driven Beat rounds through direct-store hosting and the HTTP executor. Existing paused-time tests cover timing/alarm semantics. The new snapshot equals the ready-made reference as parsed JSON; all 28 prior sample snapshots, old digests and the ready-made reference remain unchanged. Release pins consistently name 0.20.4.

Evidence: gate.py status accepts final head 48df3a0facee8de5f07d9f128ef5cf0d6c127ee7: its docs record passes, with full checks and E2E inherited from d87ee2b1fd0e5d113dc3cd32fa2e9c2279e61afd. Inspected records confirm every recorded command exited zero and only allowed Markdown changed afterward.

Not run: test suites, Docker E2E, or targeted runtime tests; inspected recorded evidence and test code, per the review brief. No repository edits outside review.py submission.
