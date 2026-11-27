Verdict: not yet

- D5: If Add saves the context and the tray write fails, retry hits the duplicate-name check. Mark the connection fields saved and switch to edit mode. Test that retry preserves intervening CLI edits.
- D4: Filtering saved `[all, old]` against `[all, new]` produces `--types all` and runs unselected `new`. Handle reserved singletons after filtering and specify the outcome.
- D2–3: Distinct config targets in one directory share `tray.toml` and take different locks. Lock the tray file’s read–modify–write by tray path or give each config separate storage.

The resolved-target check, CLI behavior and token redaction are specified. Task notes: redact header values and URL passwords like `Target`, and spell out the literal `--types all` and `none` behavior in the spec.

Strict OpenSpec validation passed. Runtime and macOS checks were not run.
