## Implementation

- [x] 1. Add worker HTTP mode selection and mode-specific settings while preserving the direct-store resolver.
  Accept server/context/token globals before and after worker, reject output flags, keep serve's refusal and the
  no-hosted-types restriction. Resolve explicit-mode conflicts, ambient store precedence, blank mode variables,
  context credentials and the applicable WorkerConfig mapping. Refuse explicit server-owned settings and do not
  parse their env/host values, unused listen settings or a store in HTTP mode. Reject on_start before config/network
  access. Add pure command/settings tests for every precedence/refusal row, invalid ignored fields, selected types,
  node_command and config-file isolation. Redact Target/settings Debug credentials/header values/URL passwords.
  Run affected App command/settings/config tests and existing store-worker tests.
- [x] 2. Add private shared Hosting builder variants and the remote-worker execution path with selected actors and
  Arc observers. Build its client using shared credential/header construction with redirects off, no added mutation
  retry and 30s total timeout alongside the existing 10s turn timeout. Implement readiness preflight and stop/start/
  stopped handling without source lifecycle calls. Test preflight with fake sources and virtual time: Unavailable
  then success, permanent failure, no request at/after 60s including after sleep, a last request crossing the horizon
  and stop during a hung request. Test registration failure cleanup and automatic-error/signal arbitration. Run
  targeted node/worker lifecycle tests; preserve local start/stop behavior and existing operator clients.
- [x] 3. Add crates/cicada-app/tests/worker_http.rs against handler-free HTTP servers with unchanged actor handlers.
  Verify actor selection and observer start/end, poisoned PostgreSQL and SQLite store env without access/file
  creation, authenticated listing/registration/GET-of-conflicting-type/leases, contract conflict diagnostics, 307
  without a redirected POST, stop during registration and running work, fatal worker exit and refused hooks with
  zero requests. Use channels and controlled I/O barriers; runtime/race assertions use virtual time as required by
  AGENTS.md, reusing the predecessor's harness where suitable. Socket framing tests need no timing-sensitive sleeps.
  Test applicable configuration errors separately from ignored server settings. Run targeted integration tests and
  `cargo check -p cicada-app --no-default-features` to prove the production path has no store-feature requirement.
- [x] 4. Update ADR 0042/index as needed, architecture's App section, app guide, crate docs, README and operations.
  Include a compiled no_run App HTTP-worker example, mode/setting precedence, startup-hook incompatibility, separate
  bootstrap, preserved context credentials on URL override, permissions for GET /types and /types/{name}, PUT
  /types/{name} and all lease routes, and cooperative shutdown/error exits. State that the preflight horizon excludes
  sequential registration; no whole-startup timeout is promised. Regenerate docs/reference/cli.md with
  CICADA_UPDATE_CLI=true, update changelog/version at build time and any version-bearing references required by drift
  checks. Run examples doctests/docs tests and CLI-reference tests. No headless-server CLI, new hook, runtime/wire
  policy or source-closing behavior belongs in this change.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate app-worker-over-http --strict`. Check cicada-app with no default features and each sqlite,
  postgres and tui feature separately; include PostgreSQL tests where available and record unavailable optional
  checks. Reconcile against the finally landed HTTP/Worker APIs, preserve existing serve/store-worker/operator tests
  and check compiled guide examples and generated-reference drift.
