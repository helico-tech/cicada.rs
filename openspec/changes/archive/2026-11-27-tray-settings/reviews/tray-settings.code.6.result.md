Verdict: fix needed

1. `crates/cicada-app/src/config.rs:302`: All readers reach the check, but `Config::load` discards the checked target and reopens the original path. Atomically retarget the symlink from a valid file to `missing.toml/` between those steps. I verified that opening the original path then returns ENOENT, which the loader turns into empty config. Tie the read to the checked target so retargeting cannot bypass validation.

`cicada-app` tests passed. Full gate and interactive AppKit were not run.
