Verdict: fix needed

1. `crates/cicada-inspector/src/appkit.rs:641` and `src/turns.rs:394`. Back can leave an already loaded turn row unselected. Open Show Actor, then go Back while the turn still runs. If the exact-address response arrives after the table renders, it sets the model selection and publishes only a pane update. Retry has the same problem. Synchronize the table selection on completion while preserving the freshly read pane. The test at `src/tests/turns.rs:257` checks the model only and lets the two responses race. Exercise both orders explicitly, including a later page bringing the row.
2. `crates/cicada-inspector/src/appkit/nodes.rs:89` and `:132`. Both heartbeat displays use `created`, which drops fractional seconds. A server timestamp ending in `30.123Z` displays as `30 UTC`. Preserve the server timestamp's precision in the table and pane, and cover a nonzero fraction in the display check.

Notes. The previous sidebar readiness assertions were removed. The three spec scenarios have model tests with bounded waits. Version pins match 0.26.0. The diff has no protected-doc edits, whitespace errors, or added lines over 120 columns.

Not run. Cargo tests, gate, and the AppKit manual checklist. The review request did not include the builder's checklist results.
