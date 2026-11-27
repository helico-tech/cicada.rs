Verdict: not yet

Blocking findings

- Design §4 calls an empty first Caused page “None”. PostgreSQL resolves child statuses after choosing ids, so a page can be empty with a next cursor. Continue by cursor before declaring the chain empty.
- Design §4 omits failed link reads. Reserve “not found” for `Ok(None)` and show request failures with Retry for the linked id.

Task notes

- Test fresh status on Forward. The proposal now names the correct endpoint. Align its Caused wording with operator retries and store-generated notices.
- Define the failed badge after an initial error with no cached count. Its scope, refresh points, loading and context reset are specified.

Snapshot restoration, context cancellation, single-flight Caused paging, same-cursor Retry, gone-parent handling and status-switch resets are covered. Strict OpenSpec validation passed. Build and UI checks were not run.
