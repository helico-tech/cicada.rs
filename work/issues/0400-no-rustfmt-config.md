---
status: done
type: debt
created: 2026-10-07
---

# 0400. No rustfmt configuration; crates are formatted inconsistently

## Problem

The workspace has no `rustfmt.toml`, and the existing crates are not `cargo fmt`-clean with either the default settings
(255 diffs in `cicada`) or `max_width = 120, use_small_heuristics = "Max"` (37 diffs in `cicada`, 14 in
`cicada-test`). Every contributor formats differently, and `cargo fmt` on a crate produces unrelated churn.
`cicada-store-sqlite` was formatted with `max_width = 120, use_small_heuristics = "Max"`, the closest match to the
existing code.

## How to reproduce

```sh
cargo fmt -p cicada --check | grep -c '^Diff'
cargo fmt -p cicada --check -- --config max_width=120,use_small_heuristics=Max | grep -c '^Diff'
```

## Notes

Proposal: add `rustfmt.toml` with `max_width = 120` and `use_small_heuristics = "Max"`, run `cargo fmt --all` once in
a separate commit, and add `cargo fmt --all --check` to the build instructions in `AGENTS.md`.

- 2026-10-07: `rustfmt.toml` (120 columns, `use_small_heuristics = "Max"`), the workspace formatted, `cargo fmt --all --check` in CI.
