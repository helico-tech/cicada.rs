Verdict: ready for tasks

Blocking findings: none.

The proposed scanner resolves the apostrophe and adjacent-address failures without changing public APIs or runtime guarantees. Its boundary rule preserves the next `://` for the outer scan, and its explicit malformed/truncated-input exclusions avoid promising general secret scrubbing. The last-at-sign rule, Unicode-safe slicing and repeated-redaction requirement are settled and testable.

Cross-checked `command::parse`, `usage`, `hide_passwords`, `App::run_with`, `AppError::usage`, settings resolution and `pick_types` against the proposed scenarios. The clap path really redacts twice; settings errors redact at AppError construction. The existing App tests provide the intended harness, including the no-start assertion. Neither adjacent proposal conflicts with this change.

Notes: In the new settings/`--types` regressions, retain a valid store setting so resolution reaches the intended failure; `resolve` checks for a store first. The existing `TempDb::env` pattern already supplies this without opening a database.

Owner questions: none.

Checks: `openspec list`, `openspec show usage-error-password-redaction`, and strict validation completed; validation passed. Read the architecture, relevant ADRs, current app spec, all change artifacts and adjacent proposals. No implementation exists yet, so implementation correctness and generated documentation drift remain for the build. No Rust tests, workspace suite or network/database checks were run.
