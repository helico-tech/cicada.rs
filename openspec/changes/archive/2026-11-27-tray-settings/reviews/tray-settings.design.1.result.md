Verdict: not yet

## Blocking findings

1. **Credentials (design.md, decisions 1 and 6).** TOML errors can quote secrets through the worker’s error chain. Sanitize shared load errors at source and redact ContextFile/entry Debug. Define path, enumeration and current accessors for CLI parity, including first-set selection.
2. **Writes (decision 4, “Another writer meanwhile”).** Atomic rename still loses overlapping CLI writes. Use a stable sidecar lock beside the resolved target across both writers’ read–modify–save. Merge dirty fields into the fresh entry, preserving headers. Define a retryable partial-save error and skip reconnect if the second write fails.
3. **Tray file (decision 2).** Specify relative and symlink placement and reject config collisions (e.g. CICADA_CONFIG=tray.toml). Quote TOML map keys. Distinguish omitted types from a saved subset filtered to zero. Refuse affected connections with a repairable error.
4. **Worker arguments (decision 3, spec requirement).** Flags override environment. Explicit All needs `--types all` over CICADA_TYPES. Inherit omits flags. Explicit concurrency 16 overrides CICADA_CONCURRENCY. Specify CICADA_SERVER/CICADA_TOKEN precedence and cover singleton actor names `all`/`none`.
5. **Validation (decisions 1 and 4).** CLI accepts a plain String. Set an Add-only rule and reject duplicates while accepting legacy names. Extract the worker’s URL check. Concurrency parses as usize with runtime minimum 1. Choose a TOML/stepper-safe ceiling.
6. **Reconnect (decision 5).** MenuControl::pick ignores the active context. Add an apply action that replaces pending picks while connecting/online/stopping, yields to Quit, and clears stale pending picks on idle save.

## Notes

- Decision 5 needs main-thread retained window ownership and accessory activation. Gate-test the model and compile AppKit on macOS. Manually check focus, masking, shortcuts, sheet cancellation and reopen on macOS.
- Five tasks fit if scoped to shared config, tray persistence/form, worker arguments, reconnect and AppKit. Split shared config first if stronger cross-file guarantees are required.

Tests and manual checks not run.
