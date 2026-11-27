Verdict: fix needed

1. `crates/cicada-app/src/config.rs:211` — The suffix check misses symlink targets. Set `CICADA_CONFIG=link` with `link -> config.toml/` (or `config.toml/.`) and an existing regular config. I reproduced `get-contexts` failing with ENOTDIR while `use-context a` succeeds and rewrites the file. Check symlink destinations before rebuilding the path and test that the file stays unchanged.

Direct paths with either suffix failed for both commands. All 44 filtered tests passed. The full gate and interactive AppKit were not run.
