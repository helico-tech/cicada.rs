## Implementation

- [x] 1. Change command::hide_passwords to keep apostrophes in candidate authorities and leave a following scheme
  for the next scan, preserving the first-colon/last-at-sign replacement and Unicode-safe slicing. Update its comment
  and AppError::Usage's comment to state the scoped guarantee and distinguish apostrophes from unsupported delimiters.
  Extend the existing helper tests with apostrophes in user/password, trailing apostrophe before @, sub-delimiters,
  percent escapes/Unicode, adjacent quoted/comma-separated addresses, empty/multiple-@ passwords and unchanged
  no-password/port/IPv6/plain-text forms. Check exact useful output for clear cases and idempotence for every fixture.
  Run `cargo test -p cicada-app --lib command`. No URL parsing dependency or Target Debug changes.
- [x] 2. Extend the actual App usage-error tests in crates/cicada-app/tests/app.rs: unexpected URL argument with
  S3'CRET, CICADA_LISTEN with apostrophes in user/password, and --types containing a passwordless URL followed by a
  password-bearing one. Use TempDb::env's valid store setting for settings failures so store selection does not mask
  the target error. Assert Usage/exit2, useful diagnostic prefixes, absence of secret fragments and the existing
  no-start property. Retain old regressions. Run `cargo test -p cicada-app --test app usage_error` and the related
  settings tests; no database/network should open on these paths.
- [ ] 3. Add the scoped usage-error clause to the app guide and the changelog entry, and choose the next patch version
  after the preceding changes when built. Avoid claiming all error text or arbitrary malformed credentials are
  scrubbed. Run `cargo test -p cicada-examples --test docs` and examples doctests if their source changed. No command,
  flag or help change is expected; verify the generated CLI reference remains current rather than rewriting it blindly.

## Finishing checks

- [ ] 4. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate usage-error-password-redaction --strict`. Confirm both real error paths pass through the helper,
  repeated redaction is stable, and no unrelated configuration/output behavior changed. Follow the build loop's
  ordinary code-review and release gates.
