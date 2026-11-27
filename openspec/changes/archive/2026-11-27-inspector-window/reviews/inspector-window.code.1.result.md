Verdict: fix needed

1. `crates/cicada-app/src/config.rs:497`, `crates/cicada-inspector/src/model.rs:178`: With `CICADA_SERVER='http://u:dummy secret@127.0.0.1:81 '`, validation succeeds, but appending `/health` makes the port invalid. A probe confirmed that both the client error and inspector redactor expose the password. Build requests from the validated URL, keep raw credentials out of errors, and correct the changelog and handoff claims.
2. `crates/cicada-inspector/src/model.rs:151`: Tokio polls the HTTP future before checking its deadline. When eight counts hang, the buffer refills on timeout and polls the remaining 42 requests after cutoff, as reproduced with pending futures. Stop admitting new requests at the deadline and preserve completed results.
3. `crates/cicada-inspector/src/tests.rs:269,305`: The deadline test requires several socket round trips within 500 ms. The context-switch test switches before the single-threaded runtime can poll the old task, then sleeps 200 ms. Synchronize on actual request starts and completions, and control time independently of socket scheduling.
4. `openspec/changes/inspector-window/docs-handoff.md:46,101,106,119` and `docs/adr/README.md:18,83` exceed 120 columns. Wrap prose and shorten the ADR table entries.

Notes: Starting counts when `/types` answers preserves the shared deadline. The incomplete CLI redactor is a poor public API for this path; use safe error summaries and keep it private. ADR scope, the tray-only dependency exception, protected docs and spec scenario coverage checked.

Checks: Inspector tests and doctest, four context-client tests, and diff whitespace passed. Main-thread guards and Close/Quit cleanup were inspected statically. Live AppKit and the full gate were not run.
