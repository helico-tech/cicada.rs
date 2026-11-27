Verdict: fix needed

1. `crates/cicada-inspector/src/tests.rs:262,340`. The deadline test still requires `/types` and its follow-up counts to finish within two wall-clock seconds. Drive the deadline on paused time after synchronizing the expected completions. The switch now waits for the old request to stall, but the 300 ms sleep can miss a delayed stale publish. Await cancellation or completion before asserting.
2. `crates/cicada-app/src/config.rs:506`. With `http://%FF:p@host` and a token, reqwest previously skipped Basic because the decoded username was invalid UTF-8. Lossy decoding now overwrites Bearer with altered credentials. Match reqwest's strict decoding and fallback while stripping all userinfo.

The original leak is fixed. Valid percent-decoded Basic credentials reached the server and beat Bearer. `percent-encoding` reuses the locked version. `hide_passwords` is crate-private again. Without the guard, the paused-clock count test would record 50 starts instead of 10. All changed lines fit 120 columns.

Passed 12 inspector tests, one doctest, six context-client tests and the diff whitespace check. Live AppKit and the full gate were not run.
