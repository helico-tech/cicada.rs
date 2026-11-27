Verdict: pass

Both round-1 findings are fixed. Running Turns synchronizes the table selection from either render path while preserving the freshly read pane. Tests cover both response orders, Retry, and a later page with or without an intervening user selection. The Nodes table and pane retain nonzero heartbeat fractions. Omitting an all-zero fraction preserves the timestamp value.

All 83 inspector tests and the doctest passed. No new blockers found. The full gate and manual AppKit checklist were not run.
