Verdict: pass

Reviewed by claude-reviewer (Claude family; built by gpt-6.2-sol).

## Blocking findings

None.

## Checked

- Parse/apply split: `NodeOverrides::parse` reads flags and variables in the old order (store, types, id, concurrency, lease TTL, heartbeat, poll, grace, max turn, lost claims, label; HTTP: refused flags, label, types, id, concurrency, poll, grace, max turn). Error texts and exit code 2 are unchanged. The apply step gives the same precedence, and HTTP keeps the host's cancel timeout.
- Hook order: serve and the store worker run parse, then `check_built_in` (same text, exit 1), then the hook, then `open_unless_stopped`. The HTTP worker runs the on_start/task refusal, `parse_http`, `http_target`, `worker_client`, then the hook. Operator commands, help and version never reach it. `take()` on one slot gives last-call-wins and one call. An Err gives `AppError::Failed`, exit 1, with its causes.
- Malformed server URL: before, the first `list_types` failed with a reqwest builder error, mapped to `SourceError::Invalid`, which was not retried: exit 1 with "the server is not ready: Listing types: builder error…". Now: exit 1 with "invalid server URL: …" or "the server URL must use http or https", before the hook. The exit code and timing are the same, and the new text is clearer and does not echo the URL. Acceptable: it follows design decision 4. No test or doc used the old text.
- Every spec scenario has a test (app.rs, worker_http.rs, and the lib.rs unit test for the missing store feature).
- No protected docs edited. CHANGELOG and handoff match the code. The handoff example's `NodeIdentity` API exists.

## Notes

1. Two new comment lines are over 120 columns: config.rs:208 (124) and lib.rs:193 (121). rustfmt does not wrap comments. Wrap them before landing.
2. tasks.md task 5 is unticked. The gate was not run, so gate evidence is still needed before push.
3. The CHANGELOG does not mention the new text for a malformed server URL. One line under Changed would cover it.
4. Only `no-url` is tested. A parseable non-http scheme (`ftp://x`) has no test.

## Run / not run

Run on ac4a433a: `cargo test -p cicada-app` (all pass), `cargo fmt --all --check` (clean), `cargo clippy -p cicada-app --all-targets --all-features` (clean), `cargo check -p cicada-app` with no features, `--features sqlite` and `--features postgres` (clean). Not run: workspace tests, the gate, E2E, `openspec validate`.
