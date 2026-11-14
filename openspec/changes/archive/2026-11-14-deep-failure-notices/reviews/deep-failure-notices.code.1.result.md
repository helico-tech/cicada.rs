Verdict: fix needed

Blocking findings:

1. `examples/src/dead_letters_upgrade.rs:176` — Validate all storage constraints before reset deletes the actors. `convert` checks typed decoding and depth, but not NUL characters. An otherwise readable export containing a NUL in a state string or key passes conversion; `reset` then deletes the original actors, and `restore` fails at `create_actor`'s NUL check. This violates the agreed requirement that invalid restore data stop the procedure before destructive operations. A focused external Rust probe against the built head confirmed that `convert` returns `Ok(())` for `{"letters":{},"extra":"\u0000"}`. Check the complete converted states against the restore constraints, including NULs and the agreed schema validation, before deletion; add a regression proving invalid exports leave every original actor and registration intact.

Notes: No other blocking findings in payload representation, notice decoding, numeric fidelity, redrive, or the reviewed scenario coverage. Version 0.17.0 and the contract changes match ADR 0043.

Not checked: I did not rerun the suite or independently reproduce the builder's PostgreSQL, isolated-consumer, lint, formatting, or feature checks. Those remain reported results; Docker E2E and cicada-app's individual SQLite/PostgreSQL feature checks were reported not run. The external probe above was the only execution experiment.
